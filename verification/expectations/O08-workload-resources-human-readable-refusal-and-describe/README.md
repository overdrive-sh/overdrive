# O08 — a mistyped workload size is refused in words, and `describe` reads the declared size back in the operator's own units

**Surface:** O (operator CLI) · **KPI:** Declared size readable in `describe` / Silent mis-size on mistyped value · **Status:** `satisfied`

<!-- Status rationale: set to `satisfied` on 2026-11-21 by an independent reviewer agent (not the implementation or capture author) after judging the verbatim captured evidence; see "Review" below. Satisfied for SHA c450168716b01ab34d7c93fce56ba95c9aadca70 and the recorded native-metal substrate only. -->

## Expectation

Ana Ortiz sizes a workload in `[resources]` with `cpu = "500m"` and
`memory = "128MiB"`. Two things hold on the built product, and no `assert!`
holds the first one:

1. **A refusal tells her what to write, not what the parser tripped on.**
   Deliberately contrasting mistakes (`memory = 134217728`, `memory = "128MB"`,
   `cpu = "500M"`, `memory = "1000B"`, the removed `memory_bytes`) each make
   `overdrive deploy` exit non-zero before any allocation exists, and the
   message **names the key or the form that was expected** (for example the
   binary units, or the lowercase milli suffix `m`). A reader who has never
   seen the grammar can correct the spec from the message alone. "Names the
   remedy" is a judgement a person makes from the captured text.
2. **A size she writes is read back in her own words.** After
   `overdrive deploy dev-box.toml`, `overdrive workload describe dev-box` shows
   `Memory:` and `CPU:` lines in the canonical spelling: `128MiB` and `500m`,
   never a raw byte count or a bare milli-unit number. A size she spelled
   differently (`memory = "1024MiB"`, `cpu = "0.5"`) is shown as `1GiB` and
   `500m`, so she can see what the platform understood.

What would refute it: a refusal whose text only says "invalid TOML" or "unknown
field `job`", or a describe that prints `134217728`.

- Anchor: docs/feature/workload-resources-human-readable/feature-delta.md AC-1.3, AC-1.5, AC-2.3, AC-2.8 (DISCUSS stories US-1, US-2)
- Anchor: docs/feature/workload-resources-human-readable/feature-delta.md D-3, D-4, DD-7
- Anchor: docs/product/architecture/adr-0339-memory-and-volume-sizes-share-one-binary-byte-size-grammar.md
- Anchor: docs/product/architecture/adr-0343-a-workload-memory-is-a-whole-number-of-mib.md

## Verification

Evidence is captured at DELIVER or DEVOPS against the built `overdrive` binary
on the native-metal substrate (`.claude/rules/testing.md` § "Native-metal
expectation captures run with `--exclusive`"): one `overdrive serve`, then
`overdrive deploy` of the accepted and refused specs above, then
`overdrive workload describe`, recording verbatim stdout, stderr and exit codes.
A point-in-time capture with a handful of deliberately contrasting specs, not a
matrix; the parser and admission rules are covered exhaustively by the test
tiers (`crates/overdrive-core/tests/acceptance/`). A reviewer who did not write
the implementation judges "names the remedy" from the captured refusal text and
sets the status.

## Evidence

Captured 2026-11-21T04:56:41Z on native-metal (x86_64, Linux 7.0.0-38-generic, `systemd-detect-virt` = none, /dev/kvm present), SHA `c450168716b01ab34d7c93fce56ba95c9aadca70`, `working_tree_dirty: true` (untracked files only, see Review), harness invocation `verification/harness/run-expectation.sh O08`, runner exit 0.

All under `evidence/`: `verification.yaml`, `substrate.txt`, `binary_under_test.txt`, `build.log`, `dirty-status.txt`, `dirty-diff.patch` (empty), `run.log`, `trace.log`, `serve.log`, `spec_*.toml` (7 specs), `deploy_<case>.{stdout,stderr,meta}` (7 cases), `describe_dev-box_poll01/02`, `describe_dev-box-gib_poll01`, `describe_{dev-box,dev-box-gib}_final.*`, `describe_refused-128mb.*`, `stop_*.*`, `leak_verdict.*.txt`, `teardown_sweep.*.txt`, `rootfs_staging.log`.

## Review

Reviewed 2026-11-21 by an independent reviewer agent (different fox; did not write the implementation or capture the evidence). Verdict: **satisfied**.

Refusals (all `overdrive deploy`, exit 2, empty stdout, verbatim stderr):

| Case | Message (key lines) | Names key/form/remedy |
|---|---|---|
| `memory = 134217728` | `invalid type: integer `134217728`, expected a byte size string such as 128MiB (units: B, KiB, MiB, GiB, TiB)` | yes: a string with an example |
| `memory = "128MB"` | `invalid byte size "128MB": decimal units are not accepted; use the binary units KiB, MiB, GiB or TiB` | yes |
| `cpu = "500M"` | `invalid cpu quantity "500M": unknown suffix "M"; the only suffix is the lowercase milli suffix m, as in 500m` | yes |
| `memory = "1000B"` | `invalid job spec: field `memory`: memory must be a whole number of MiB; got 1000B` | yes: names the key and the form (whole MiB); does not give a corrected value, but the form is stated |
| `memory_bytes` | `unknown field `memory_bytes`, expected `cpu` or `memory`` | yes: lists the valid keys |

Before any allocation exists: `describe_refused-128mb` returns `control plane returned HTTP 404: not_found (workloads/refused-128mb)` (exit 1) after the refused deploy. Evidenced for one refused case only; the other four fail on the same spec-parse path before intent is written, but were not each described. Accepted as sufficient for a point-in-time capture.

Describe read-back: `dev-box` (`cpu = "500m"`, `memory = "128MiB"`) prints `Memory:        128MiB` and `CPU:           500m`; `dev-box-gib` (`cpu = "0.5"`, `memory = "1024MiB"`) prints `Memory:        1GiB` and `CPU:           500m`. No raw byte count or bare milli number appears.

Provenance: dirty state is limited to untracked paths (`dirty-status.txt` lists only `.develop-progress.json`, the O08 evidence dir, `execution-substrate`, `runner.sh`); `dirty-diff.patch` is empty, so no tracked source differed from the SHA and the binary (built with default features from the synced tree, `build.log` shows a clean compile) is the SHA's code. None of the untracked files is compiled into the binary. Binary identity is the path and `--version` only; no hash was recorded.

Caveats, none touching the claim: both accepted workloads ended `Failed (backoff exhausted)` because the guest's transport setup failed (`creating the transport dummy device failed: Not supported`), an environment matter unrelated to resources; the `Memory:`/`CPU:` lines print after the attempts table; a reader of the evidence for later source states must re-capture (satisfied for this SHA only). Candidate to stabilize into a test per `.claude/rules/verification.md` § Stabilize.
