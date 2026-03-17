# IR Observational Benchmarks

## Purpose

The IR observational benchmark corpus is a checked-in set of small, self-contained
XLS IR examples used to measure proof cost in the `PIR -> Lustre -> Kind2` path.

These samples are not end-user correctness demos. They are engineering benchmarks
meant to answer questions like:

- does an exporter change help narrow-control cases?
- does it help wide-output cases?
- does it help wide-state cases?
- does an assumption or passthrough cone dominate the step obligation?

The current corpus lives under:

```text
examples/demos/ir_observational/
```

and is driven by:

```text
examples/ir_observational_bench.rs
```

## Expected Use

The intended workflow is:

1. make a proof-generation or exporter change
2. run one or more IR observational benchmarks
3. compare `init relation` and `step obligation` times
4. decide which benchmark shapes improved or regressed

## What The Benchmarks Prove

Each benchmark runs a decomposed self-proof:

- `init relation`
- `step obligation`

The runner loads one IR file, exports the same `step` function on both sides,
and proves:

- the initial relation holds from zero init
- one step preserves the relation and output agreement

So these are not optimization comparisons. They are controlled cost probes for
the current proof shape.

## Running The Benchmarks

Run the commands from the repository root with the tools listed in the
[README prerequisites](../README.md#prerequisites).

List the available samples:

```bash
cargo run --locked --example ir_observational_bench -- --list
```

Run the whole corpus in one sweep:

```bash
cargo run --locked --example ir_observational_bench -- --all
```

Run one sample:

```bash
cargo run --locked --example ir_observational_bench -- batch_control
```

Show Kind2 progress lines:

```bash
cargo run --locked --example ir_observational_bench -- --show-progress batch_outputwide
```

Emit machine-readable CSV for a sweep:

```bash
cargo run --locked --example ir_observational_bench -- --all --format csv
```

Emit machine-readable JSON for a sweep:

```bash
cargo run --locked --example ir_observational_bench -- --all --format json
```

## Benchmark families

The `batch_*` samples model a synthetic countdown controller. A begin signal
loads the requested length while idle. Each advance decrements the remaining
count until the batch completes.

- `batch_control` reports activity and completion from a 7-bit counter.
- `batch_tag` adds a derived 4-bit tag to the output.
- `batch_contract` assumes a 12-bit annotation stays stable during an active batch.
- `batch_passthrough` exposes the annotation in the output under the same assumption.
- `batch_passthrough_gated` exposes the annotation only while active.
- `batch_outputwide` adds a 96-bit output field.
- `batch_statewide` adds a 96-bit accumulator to the state.
- `batch_wide` exercises both the 96-bit accumulator and a wide output.

The hazard samples vary queue behavior, pending-register width, and history depth.

- `hazard_control`: compact control-only recurrence for queue/backpressure behavior
- `hazard_scan2`: tiny synthetic reg-hazard kernel with decode plus history scans
- `hazard_scan4`: small synthetic reg-hazard kernel with decode plus history scans
- `hazard_scan5`: small synthetic reg-hazard kernel with decode plus history scans
- `hazard_scan6`: small-medium synthetic reg-hazard kernel with decode plus history scans
- `hazard_scan7`: medium synthetic reg-hazard kernel with decode plus history scans
- `hazard_scan8`: medium-small synthetic reg-hazard kernel with decode plus history scans
- `hazard_scan9`: medium synthetic reg-hazard kernel with decode plus history scans
- `hazard_scan10`: medium synthetic reg-hazard kernel with decode plus history scans
- `hazard_scan11`: medium-large synthetic reg-hazard kernel with decode plus history scans
- `hazard_scan12`: medium-large synthetic reg-hazard kernel with decode plus history scans
- `hazard_scan14`: near-threshold synthetic reg-hazard kernel with decode plus history scans
- `hazard_scan16`: small synthetic reg-hazard kernel with decode plus history scans
- `hazard_scan64`: medium-width synthetic reg-hazard kernel with decode plus history scans
- `hazard_scan256`: wide synthetic reg-hazard kernel with decode plus history scans
- `hazard_scan256_hist8`: same as `hazard_scan256` but with deeper history arrays
- `hazard_scan256_factors`: same as `hazard_scan256` but with richer factorized output

The `hazard_scan*` steps use `decode` and dynamic bit tests on a pending-register
vector, then scan recent writeback entries. Compare the numbered samples to
measure the effect of vector width. Compare `hazard_scan256` with
`hazard_scan256_hist8` to measure the effect of history depth at a fixed width.

## How To Read Results

The main values to watch are:

- `init relation wall time`
- `step obligation wall time`
- `Kind2 result` for each phase

For `--format csv` and `--format json`, the runner emits:

- demo metadata
- state/output widths
- init-phase status, wall time, and runtime line
- step-phase status, wall time, and runtime line

That output is intended to make before/after exporter comparisons scriptable.

## Comparing worktrees

Create a baseline at the current commit before changing the exporter in your
working tree. These Linux helpers use Bash, GNU `time` at `/usr/bin/time`,
`grep`, and `sed`.

```bash
git worktree add --detach ../xlsynth-kind2-baseline HEAD
```

After making a candidate change in the original worktree, compare two
countdown benchmarks against the baseline.

```bash
scripts/compare_ir_observational_bench.sh \
  --baseline-dir ../xlsynth-kind2-baseline \
  --samples batch_control,batch_tag
```

The companion script can compare the same decomposed proof in both
worktrees. The step function and its state relation live in separate IR
files. Paths after `--` are relative to each worktree.

```bash
scripts/compare_ir_observational_equiv.sh \
  --baseline-dir ../xlsynth-kind2-baseline \
  -- examples/demos/ir_observational/hazard_control_step.ir __hazard_control__step \
     examples/demos/ir_observational/hazard_control_step.ir __hazard_control__step \
     --relation-ir examples/demos/ir_observational/hazard_control_rel.ir \
     --relation-top __hazard_control_rel__rel --decompose-relation
```

The benchmark comparison writes:

- one log per `(label, slug)` pair
- a `summary.tsv` with the repo path, exit code, total wall time, and the raw
  benchmark CSV row

Useful flags:

- `--samples slug1,slug2,...`: focus on the benchmark shapes you care about
- `--timeout-seconds N`: cap each invocation using GNU `timeout`
- `--show-progress`: preserve Kind2 progress lines in each per-run log

In practice:

- fast `init` and slow `step` usually means the inductive next-state obligation is the bottleneck
- a regression only on `*_outputwide` suggests output or observation cost
- a regression only on `*_statewide` suggests relation or next-state cost
- a regression on `*_contract` or `*_passthrough` suggests assumption-domain handling

## Adding New Samples

When adding a new benchmark:

1. put a self-contained IR file in `examples/demos/ir_observational/`
2. include at least:
   - `top fn step(...) -> (...)`
   - `fn rel(...) -> bits[1]`
   - optional `fn env(...) -> bits[1]`
3. register it in `examples/demos/ir_observational/mod.rs`
4. name the file, package, and slug after the behavior being measured

Guidelines:

- prefer generic names like `batch_*`, `controller_*`, or `buffer_*`
- keep the sample focused on one cost shape
- document the intended shape in the sample description

## Relationship To Other Examples

This corpus is separate from the DSLX observational demos in:

```text
examples/demos/observational/
```

Those DSLX demos are semantic examples for observational equivalence.

The IR corpus is a proof-cost benchmark set for exporter and harness work.
