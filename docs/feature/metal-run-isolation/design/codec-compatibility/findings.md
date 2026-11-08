# Quint recording codec compatibility

The ordinary `{ tag, value: { actor, value } }` record reaches the installed quint-connect 0.1.2 public runner's `Driver::step` correctly. No new sum type, module, action, production interface or owner mechanism is required. Only the existing model-only recording representation changes, including recording `Idle` explicitly when `REC = true`.

## Boundary and diagnosis

The project-local role definition `.claude/agents/quint-modeler.md` was read. This is the architect's bounded recording-codec delegation for P2/P5, preserving P1–P5's production contracts. The diagnostic does not drive Metal, shell admission, systemd, a kernel namespace or a production state projection.

Installed primary source is `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/quint-connect-0.1.2/`:

- `src/driver/mod.rs`: `Config::nondet` is a static record path; an empty path selects the built-in MBT variables.
- `src/driver/step.rs:24`: private `Step::new` selects the sum-type-shaped decoder for a nonempty nondet path.
- `src/driver/step.rs:120`: that decoder requires a string `tag` and a record `value` (or unit payload); it does not require a Quint variant declaration.
- `src/runner/mod.rs:71–80`: the runner constructs `Step` before calling `Driver::step`, so a handler-side rename cannot fix the old flat record.
- `src/trace/generator/test.rs` and `run.rs`: the public runner executes real `quint test` / `quint run` and reads their ITF files. No replacement parser or fabricated trace was used for decoding.

The actual initial state from the unchanged model's `independent_worktrees` named trace contains `{actor: 0, op: "init", value: 2}` at the qualified key `runner_conf::runner_protocol::conf_step`. It has no MBT variables. Public `runner::run_test` fails with `Expected action to be a sum type variant` before any `Driver::step` call. See [original ITF](original-flat-0.itf.json), [runner log](original-flat-runner.log) and [zero reached steps](original-flat-driver.json). This is an expected negative codec result, not a production failure.

## Exact driver mapping

```rust
Config {
    state: &[],
    nondet: &["runner_conf::runner_protocol::conf_step"],
}
```

The qualified variable name is one path component. Each decoded step supplies `action_taken = tag`, plus integer picks named `actor` and `value` from the inner record. The [permanent model README](../../../../../specs/quint/metal-run-isolation/README.md#model-only-replay-record) lists the exact unchanged action/pick mapping. In particular `init` supplies actor 0/value `c`; `Sync` supplies actor `c`/value 1; `Maintain` supplies actor 0/value `op`; and `Idle` supplies actor 0/value 0. The actual production-derived state projection must continue to exclude `conf_step` and all other declared model-only fields.

## Actual trace and public-runner results

Seed: **20261108**. Quint: **0.32.0**. quint-connect: **0.1.2**.

| Population | Actual states reaching `Driver::step` | Result |
|---|---:|---|
| All 13 existing named runs, one sample each | 119 | Every action and both integer picks exactly match the corresponding actual ITF state |
| Four randomized traces, maximum 32 transitions each | 132 | Every action and both integer picks exactly match; all 132 recorded tags also match real `mbt::actionTaken` |
| `Idle` within the random population | 45 | Every Idle decodes actor 0/value 0, including after non-Idle predecessors |

[Codec results](codec-results.json), [random MBT cross-check](random-mbt-mapping.json), [exact public-runner commands and exit codes](commands.json), and the adjacent named/random ITF plus driver reports retain the bounded evidence. [Original/repaired comparison](named-before-after.json) additionally verifies all 13 original and repaired named traces have identical non-recording state and identical action/pick meanings.

The [throwaway driver source](codec-probe.rs), [standalone manifest](codec-probe.Cargo.toml), [dependency lock](codec-probe.Cargo.lock), and [diagnostic reproduction script](verify_codec.py) are verification artifacts only. The driver uses `State = ()`, deliberately disabling state comparison. It proves library/record compatibility, **not production conformance**, admission correctness, cleanup correctness or the honesty of a future owner's projection. No diagnostic is registered in a product test suite or in `checks.toml`.

To reproduce, place the source at a temporary crate's `src/main.rs`, copy the adjacent manifest and lock as `Cargo.toml` and `Cargo.lock`, build it, and invoke `python3 verify_codec.py <temporary-crate>/target/debug/metal-quint-codec-probe`. The script calls real Quint and the public runner; the runner constructs the private `Step` internally. The actual initial reproduction used the temporary workspace recorded in [workspace.json](workspace.json).

## Protocol preservation

[The complete recording diff](recording-only.diff) changes only `ConfStep`, its six pre-existing write sites and the existing Idle action's metadata write. No guard or non-recording assignment changes. Removing the `ConfStep` declaration and `conf_step` next-state assignments makes the original/repaired model text identical modulo whitespace, including every action, invariant, temporal property, fairness tuple and named run. [Mechanical preservation evidence](preservation.json) also records byte-identical `checks.toml`, the unchanged hazard hash, and the unchanged 13 named runs.

`REC = false` retains constant recording metadata after initialization. Its initial flat and nested records are a bijective encoding of the same two checkout choices, so exhaustive states have the same transition relation under that encoding. No production state or projection is added. No new protocol, action, invariant, property, hazard or check is introduced.

No underspecified error contract or additional environment assumption was needed for this recording-only correction. The existing README assumption-to-native-validation mapping remains unchanged. No native construction campaign is needed to establish the local library's record shape; DISTILL still owns the actual Metal conformance tests.

## Formal gate and tool-written evidence

All **53 repository Quint files typechecked**. All eighteen affected manifest checks passed individually before the final record. The final exact command was:

```text
cargo xtask quint check --subsystem metal-run-isolation --record --jobs 3 --tlc-workers 1 --tlc-heap 1g
cargo xtask quint verify-evidence --subsystem metal-run-isolation
```

Both exited **0**. Final record: **16 holds, 2 expected hazard violations, 0 unexpected results**, including all thirteen unchanged named runs. The tool wrote the complete permanent evidence and verified the current source hashes. It also confirmed no Quint/JVM process from the record run remained. No evidence file was hand-edited. See [record output](final-record.log), [verify-evidence output](final-verify-evidence.log), [repository typecheck](repository-typecheck.log), [individual affected checks](affected-checks.json), and [formal counts/versions/hashes](formal-validation.json).

The safety and each weak-fair progress check explored **695,458 distinct states / 19,473,450 generated states**, exactly matching the pre-correction run under the constant-metadata encoding. The retained five `final-*-tool.log` files are copies of the existing tool's actual logs, including both expected hazard counterexamples. Bounds remain two invocation slots, two possible checkout identities and one maintainer.

Final timings were safety 52.8s, owner progress 117.7s, maintenance progress 83.4s; total wall time 284.2s. One preliminary owner-progress check took 151.6s and emitted the existing CI-duration warning; the final record was within 120s. The eighteen-entry manifest, including its existing CI flags, is unchanged as required by this bounded task. No timeout or unexplained outcome occurred.

No commits, production changes, DISTILL edits, new formal checks or additional agents were created. Existing dirty service DELIVER work was preserved.
