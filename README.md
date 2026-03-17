# xlsynth-kind2

Prototype workspace for a `DSLX -> XLS IR -> Lustre -> Kind 2` flow aimed at
XLS-style step functions of the form `fn f(i, s) -> (o, s)`.

The intended frontend is a checked-in DSLX `.x` file. `xlsynth` compiles that
DSLX into XLS IR text, `xlsynth-pir` parses the IR structurally, and the next
stage lowers that parsed IR into a Lustre model plus any support-library glue
needed for Kind 2. The repo now also has an equivalence lane that exports the
same step function to both XLS SMT-LIB and Kind2-generated SMT-LIB, then asks
Z3 to prove the two encodings agree on `(o, s_next)`.

## Prerequisites

The commands below run from the repository root on x86-64 Linux. Install
rustup, a C/C++ build toolchain, and `curl`. Rustup selects the nightly pinned
in `rust-toolchain.toml` when you run Cargo here. The crate uses Rust edition
2024, and its locked `xlsynth-pir` dependency uses the nightly-only
`map_try_insert` feature. A fresh build downloads the toolchain, locked crates
from crates.io, and native XLS release artifacts from GitHub.

For Kind2 proofs, use a running Docker daemon accessible to the current user.
The runner pulls the pinned Linux/amd64 Kind2 3.0.0 image on first use. To use
a local Kind2 binary such as 2.3.0, set `XLSYNTH_KIND2_BIN` to its path and
install Z3 on the host. The SMT roundtrip tests and fuzz target also call
host Z3, including when Kind2 runs in Docker. Put `z3` on `PATH` or set
`XLSYNTH_Z3_BIN` to its path for the roundtrip checker.

## Quick start

Print the XLS IR and recurrence signature for the checked-in DSLX accumulator.

```bash
cargo run --locked --example accumulator
```

Prove that an 8-bit accumulator and its widened 16-bit implementation produce
the same output trace. The demo reports `Kind2 result: valid`.

```bash
cargo run --locked --example observational_equiv -- add_vs_widening
```

Run the smallest countdown benchmark. It checks that the initial states
satisfy the relation and that one step preserves the relation and equal outputs.
Both phases should report `valid`.

```bash
cargo run --locked --example ir_observational_bench -- batch_control
```

For the full benchmark list, CSV output, and worktree comparisons, see the
[IR benchmark guide](docs/ir-observational-benchmarks.md).

### Staged proof

The checked-in plan first proves that two copies of the queue controller
agree on their credit count and active flag. Its main theorem uses that lemma
to prove that their output traces agree.

```bash
cargo run --locked --example ir_observational_equiv -- \
  examples/demos/ir_observational/hazard_control_step.ir __hazard_control__step \
  examples/demos/ir_observational/hazard_control_step.ir __hazard_control__step \
  --proof-plan examples/demos/ir_observational/hazard_control_plan.toml
```

The `[[aux]]` entry names the relation helper and its IR file relative to the
plan. `[main].depends_on` names the lemma the main proof may use.

## Observational equivalence demos

List the checked-in DSLX pairs with `cargo run --locked --example observational_equiv -- --list`.

Current checked-in observational demos:

| Demo | Expected result | Description | File |
| --- | --- | --- | --- |
| `add_vs_widening` | `valid` | direct u8 accumulation versus widened u16 state accumulation with truncation | `examples/demos/observational/add_vs_widening.x` |
| `delay_vs_widened_delay` | `valid` | one-step input delay with u8 state versus widened u16 delayed state | `examples/demos/observational/delay_vs_widened_delay.x` |
| `lsb_toggle_vs_counter` | `valid` | u1 toggling state versus u8 counter whose low bit carries the same output trace | `examples/demos/observational/lsb_toggle_vs_counter.x` |
| `shifted_input_vs_widened_shifted_input` | `valid` | shifted u8 input versus widened u16 shift followed by truncation | `examples/demos/observational/shifted_input_vs_widened_shifted_input.x` |
| `binary_vs_one_hot_latched_detector` | `valid` | classic three-state detector with binary encoding versus one-hot encoding | `examples/demos/observational/binary_vs_one_hot_latched_detector.x` |
| `unreachable_state_elimination` | `valid` | parity FSM with an unreachable dead state versus the eliminated-state machine | `examples/demos/observational/unreachable_state_elimination.x` |
| `equivalent_state_merge` | `valid` | FSM with two equivalent states versus a merged-state implementation | `examples/demos/observational/equivalent_state_merge.x` |
| `add_vs_offset_widening` | `invalid` | same recurrence shape as `add_vs_widening`, but rhs output is offset by one | `examples/demos/observational/add_vs_offset_widening.x` |

One example pair:

```text
examples/demos/observational/add_vs_widening.x

f(i: u8, s: u8) -> (u8, u8) {
     let o = i + s;
     let next_s = s + 1;
     (o, next_s)
}

g(i: u8, s: u16) -> (u8, u16) {
     let sum = zero_ext(i) + s;
     let o = low_bits(sum);
     let next_s = s + 1;
     (o, next_s)
}
```

The demo runner proves that `f` and `g` are observationally equivalent.
They carry different state types, but with zero initialization they produce
the same `o` sequence for every shared input stream.

If you intentionally pick a non-equivalent pair, the runner prints the
counterexample trace from Kind2:

```text
$ cargo run --example observational_equiv -- add_vs_offset_widening
demo: add_vs_offset_widening
requested demo: add_vs_offset_widening
description: same recurrence shape as add_vs_widening, but rhs output is offset by one
proof property: outputs_match
expected result: invalid
Kind2 result: invalid
Kind2 wall time: 2.335s
counterexample:
Counterexample:
  Node main ()
    == Inputs ==
    i                          0
    == Outputs ==
    o_a                        0
    o_b                        1
    outputs_match          false
    == Locals ==
    s_a                        0
    s_a_next                   1
    s_b                        0
    s_b_next                   1
    input_history_in_range  true
    input_in_range          true
```

### Historical timings

The following measurements are from an earlier workspace run. The runner
measured elapsed time inside the Kind2 invocation. The full `cargo run`
measurements also include example startup.

| Demo | State widths | Property | Closing `k` | Kind2 wall time | Full `cargo run` wall time |
| --- | --- | --- | --- | --- | --- |
| `add_vs_widening` | `8` vs `16` | `o_a = o_b` | `2` | `2.361s` | `2.54s` |
| `delay_vs_widened_delay` | `8` vs `16` | `o_a = o_b` | `2` | `2.093s` | `2.45s` |
| `lsb_toggle_vs_counter` | `1` vs `8` | `o_a = o_b` | `1` | `1.561s` | `1.72s` |
| `shifted_input_vs_widened_shifted_input` | `8` vs `16` | `o_a = o_b` | `1` | `2.178s` | `2.57s` |
| `binary_vs_one_hot_latched_detector` | `2` vs `4` | `o_a = o_b` | `4` | `2.194s` | `2.58s` |
| `unreachable_state_elimination` | `2` vs `1` | `o_a = o_b` | `1` | `3.823s` | `4.14s` |
| `equivalent_state_merge` | `2` vs `1` | `o_a = o_b` | `3` | `1.716s` | `1.91s` |

The `k` values above are observed values from one run. Kind2 runs multiple
engines in parallel, so the winning engine and reported `k` can shift slightly
across runs even when the proof remains easy.

## Tests and fuzzing

Run the Rust tests:

```bash
cargo test --locked
```

Run the roundtrip equivalence test directly:

```bash
cargo test --locked accumulator_roundtrip_matches_xls_smt -- --nocapture
```

Build and run the reference Kind 2 proof in Docker:

```bash
docker build -t xlsynth-kind2-proof .
docker run --rm xlsynth-kind2-proof
```

Install the `cargo-fuzz` subcommand to run the fuzz target on the pinned nightly.

```bash
cargo fuzz run step_roundtrip_equiv
```

Inspect which live IR ops the current fuzz generator is exercising:

```bash
cargo run --locked --example ir_fuzz_hitlist
```

## Repo contents

- `dslx/accumulator.x`: the source DSLX step function we compile into XLS IR.
- `examples/accumulator.rs`: compiles the DSLX file with `xlsynth`, parses the
  resulting IR with `xlsynth-pir`, and validates the recurrence signature.
- `examples/observational_equiv.rs`: proves that two differently written step
  functions produce the same output sequence even though their state widths
  differ.
- `examples/demos/observational/`: checked-in DSLX demo pairs used by the
  observational-equivalence example and timing table.
- `examples/ir_observational_bench.rs`: runs the checked-in IR observational
  proof-cost benchmark corpus.
- `examples/demos/ir_observational/`: checked-in IR benchmark samples used to
  measure decomposed proof cost across different state/output/assumption shapes.
- `scripts/compare_ir_observational_bench.sh`: runs and times selected
  benchmarks in a baseline and candidate worktree.
- `scripts/compare_ir_observational_equiv.sh`: helper for comparing one custom
  `ir_observational_equiv` proof invocation across two worktrees or revisions.
- `examples/ir_fuzz_hitlist.rs`: samples `xlsynth-pir::ir_fuzz` recurrence-shaped
  IR functions and reports which live ops the exporter still misses.
- `examples/ir_equiv_outcomes.rs`: replays saved fuzz corpus inputs and
  summarizes whether they are equivalent, unknown, disproved, or skipped.
- `proofs/basic_recurrence.lus`: a hand-written reference for the target Lustre
  recurrence shape we want property proofs to use.
- `Dockerfile`: builds a container that runs Kind 2 on the checked-in Lustre
  reference proof.
- `src/pipeline.rs`: the DSLX/XLS IR frontend.
- `src/pir_lustre.rs`: lowers a narrow PIR subset into a pure Lustre `step`
  node.
- `src/observational.rs`: pairs two exported step nodes into a combined Lustre
  model with an observational-equivalence property.
- `src/xls_smt.rs`: emits SMT-LIB for the original XLS IR function via
  `xlsynth-sys`.
- `src/kind2_smt.rs`: runs Kind 2 and captures `kind2_sys.smt2`.
- `src/equiv.rs`: wraps the Kind2 SMT step equations back into BV form and
  checks equivalence against XLS SMT in Z3.
- `fuzz/fuzz_targets/step_roundtrip_equiv.rs`: uses `xlsynth-pir`'s
  `FuzzSample` generator to build small XLS IR step functions and runs them
  through the full roundtrip.
- `docs/design-strategy.md`: compact design note for the lowering model and
  roundtrip validation strategy.
- `docs/ir-observational-benchmarks.md`: usage note for the IR observational
  benchmark corpus and how it should be used when tuning exporter/proof cost.

## Current status

- The DSLX frontend is real: the crate compiles `.x` source to XLS IR text and
  parses it as PIR.
- The recurrence-shape check is real: the crate verifies that the selected top
  function has the form `(input, state) -> (output, state)`.
- The PIR-to-Lustre exporter is real for a growing subset driven by the live
  IR fuzz frontier, including arithmetic, comparisons, shifts, tuple plumbing,
  slices, concat, and selected bit-twiddly structural ops.
- The roundtrip equivalence check is real for that same supported subset:
  `DSLX -> XLS SMT` versus `DSLX -> PIR -> Lustre -> Kind2 SMT`.
- There is now a relational proof path for output traces: two exported step
  nodes can be composed into one Lustre recurrence, each with its own state
  width and constant init value, and Kind2 can prove `o_a = o_b` inductively.
- The fuzz lane is now IR-native: `xlsynth-pir::ir_fuzz` generates the XLS IR
  samples directly, then the crate proves export preservation on those samples.
- The remaining work is broadening the lowering subset and adding a proper
  support library for richer XLS IR semantics.

## License

This project is licensed under the [Apache License, Version 2.0](LICENSE).
