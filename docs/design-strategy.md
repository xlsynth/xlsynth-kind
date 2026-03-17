# Design Strategy

## Goal

Compile a DSLX step function of the form:

```text
fn f(i, s) -> (o, s_next)
```

into:

- XLS IR
- a Lustre step node for model checking
- SMT artifacts for equivalence checking

The key requirement is semantic preservation across export.

## Current Shape

The current pipeline is:

```text
DSLX -> XLS IR -> xlsynth-pir parse/traverse -> Lustre export
```

For fuzzing, we also start directly from generated XLS IR:

```text
xlsynth-pir FuzzSample -> XLS IR -> Lustre export
```

## Lustre Encoding

Kind2/Lustre gives us:

- `bool`
- `int`
- `real`

For proofs, Lustre `int` is mathematical unbounded integer, which is exactly what we want.

We encode XLS unsigned bitvectors `uN` as Lustre `int` with the invariant:

```text
0 <= x < 2^N
```

This means export is mostly a matter of expressing XLS bitvector semantics using integer formulas plus masking/range restoration.

## Core Lowering Rule

For each XLS `uN` value:

- represent it as Lustre `int`
- preserve the canonical range after every operation

Examples:

- add: `(a + b) mod 2^N`
- sub: `(a - b) mod 2^N`
- mul: `(a * b) mod 2^N`
- not: `(2^N - 1) - x`
- slice: `(x div 2^lo) mod 2^width`
- concat: `x * 2^m + y`

Comparisons operate over the integer encoding directly when that matches XLS unsigned semantics.

## Why This Works

Because Lustre `int` is unbounded, we do not have host overflow concerns inside the proof engine.

So the main proof obligations are:

1. every exported value stays in canonical range
2. every exported operation matches the intended XLS bitvector semantics

This is the role of the support library / helper formulas.

## Validation Strategy

We want translation validation, not just trust in the exporter.

The intended validation lanes are:

1. Differential execution
- compare the original XLS function and the exported model on random inputs

2. SMT equivalence
- emit SMT for the original XLS function
- emit SMT for the exported form
- build a miter that asks whether `o` or `s_next` can differ on the same `i, s`
- expect `unsat`

3. Property proving
- wrap the step node as a recurrence / time-sequence harness
- prove inductive properties in Kind2

## Fuzzing Strategy

The current fuzz direction is:

```text
xlsynth-pir FuzzSample
  -> XLS IR step function
  -> Lustre export
  -> Kind2 SMT
  -> equivalence check vs XLS SMT
```

Meaning:

- `unsat` = export preserved semantics
- `sat` = bug
- unsupported sample = skip

This is meant to widen coverage based on real generated IR, not just hand-written examples.

## Near-Term Direction

The next architectural cleanup should be:

```text
DSLX / XLS IR -> shared step semantics -> {Lustre, SMT, Rust evaluator}
```

That gives us one semantic source of truth and lets us check:

- XLS SMT vs our SMT
- our SMT vs Lustre-derived semantics
- recurrence properties in Kind2

## Scope Notes

The current focus is the pure step function:

```text
f(i, s) -> (o, s_next)
```

The recurrence harness is a separate layer on top of that step semantics. That keeps semantic equivalence checking simpler and lets us fuzz the step function directly before proving temporal properties.
