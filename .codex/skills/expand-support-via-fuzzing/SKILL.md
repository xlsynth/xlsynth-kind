---
name: expand-support-via-fuzzing
description: Run the xlsynth-kind2 IR fuzz/equivalence loop, identify the next unsupported or failing XLS IR operation from live samples, add exporter support with a focused unit test, verify with tests and fuzzing, then commit and push.
---

# Expand Support Via Fuzzing

Use this skill when the goal is to widen XLS IR to Lustre export coverage based on real fuzz traffic.

## Workflow

1. Inspect current state.
- Run `cargo run --example ir_fuzz_hitlist` to find live unsupported operations.
- Run a short `cargo fuzz run step_roundtrip_equiv -- -runs=N` pass to see whether supported samples fail equivalence.

2. Pick one target.
- Prefer the highest-signal unsupported operation from the hitlist.
- If there are no unsupported operations in the sampled frontier, pick the first real equivalence failure instead.
- Keep each pass to one operation family or one tightly related set of cases.

3. Add coverage surgically.
- Update the lowering path in `src/pir_lustre.rs`.
- Update equivalence lowering in `src/equiv.rs` if the new Lustre form needs SMT bridging support.
- Add or extend a focused unit test for the new operation.

4. Verify.
- Run `cargo test`.
- Re-run `cargo run --example ir_fuzz_hitlist`.
- Re-run a short `cargo fuzz run step_roundtrip_equiv -- -runs=N`.

5. Land the change.
- Commit only the files for that support expansion.
- Push the branch.

## Notes

- Treat unsupported samples as skips, not bugs.
- Treat solver `sat` results as semantic bugs.
- Prefer small, deterministic tests for the new operation family before relying on fuzzing.
- Keep the loop short; the point is steady coverage expansion, not a giant batch.
