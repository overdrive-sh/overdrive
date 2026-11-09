# Queued controller EOF — post-correction native owner conformance

## Result and boundary

The exact previously failing **0x8e8e19a4** now passes through the corrected
production durable helper, together with the existing before-START EOF and
after-START controller-loss controls. A separate default-seeded randomized
execution also passes, at fresh seed **0x83bfe04**. Both ordinary canonical
Metal commands return **0**. These are four test executions across three
identities; this capture does **not** claim a fresh all-fifteen result or
completion of native placement/guest-lane acceptance.

The immutable pre-correction failure remains in
[`owner-queued-eof-gap-2026-11-09-073342Z`](../owner-queued-eof-gap-2026-11-09-073342Z/README.md).
The production correction is `8b1dd117d9ea020c6fb0429896f7a32186574331`, independently
approved by the focused P2 review in iteration 22. No production or model file
was changed for this verification.

## Exact commands and retained captures

Each directory retains the complete combined foreground output, actual
invocation/source/dirty-state receipt, and exact test-driver diff used for
that execution. The six raw files are byte-identical to their `/tmp` sources.

| Capture | Native outer run | Selected executions | Outcome |
|---|---|---|---|
| `seeded/` from `/tmp/metal-owner-queued-eof-20261109T090213Z-seeded` | `c660f27ac6cee37876c896d0` | Randomized at 0x8e8e19a4; before-START EOF; after-START controller loss | 3 passed; aggregate 0 |
| `fresh/` from `/tmp/metal-owner-queued-eof-20261109T090440Z-fresh` | `ee2d7e992c30a14ce54e3ece` | Randomized with default seed selection, yielding 0x83bfe04 | 1 passed; aggregate 0 |

```sh
cargo xtask metal run -- cargo nextest run -p xtask --lib --features integration-tests -E 'test(=metal_run_isolation_conformance::protocol_randomized_owner_traces) | test(=metal_run_isolation_conformance::protocol_controller_eof_before_start) | test(=metal_run_isolation_conformance::protocol_controller_loss_after_start)' --no-capture
cargo xtask metal run -- cargo nextest run -p xtask --lib --features integration-tests -E 'test(=metal_run_isolation_conformance::protocol_randomized_owner_traces)' --no-capture
```

The existing private `quint_run` seed attribute was temporarily set only for
the first command and restored before the second. No public environment key,
config, model action, projection, timeout, test identity, or oracle changed.
Both randomized executions use the existing four traces and 24-step bound.
The named controls retain their original named-run oracles. The commands run
the existing executable tests; no separate model-check campaign was run.

## EOF completes while the independent maintainer still owns EX

The seeded log records actor 1's actual runner terminal
`controller_eof_before_start`, with its real unit deactivating and MainPID 0,
while the independently observed maintainer still holds the **exclusive**
kernel flock on the same fixture barrier:

| Actual observation at actor 1 EOF completion | Value |
|---|---|
| Ordinary run | `b92b366452bf37e5f5e9b27d` |
| Ordinary terminal | `kind=runner`, `error=controller_eof_before_start` |
| Ordinary unit | loaded, deactivating, MainPID 0 |
| Maintainer run | `be0a9867709b059b090d9b89` |
| Maintainer unit | loaded, active, MainPID 2517636 |
| Maintainer retained phase / actual barrier mode | `waiting_for_lease` / **exclusive** |
| Maintainer terminal | absent (`NotFound`) |

The per-step comparison accepts exactly the unchanged model poststate:
ordinary phase 5/terminal 75 while maintenance remains 1. The actual
maintainer's retained phase has not yet advanced to preparing in this sample;
its real exclusive fdinfo is the admission fact, as required by the existing
projection. No maintenance START, completion, release, or driver-ledger
substitution is needed for the ordinary EOF terminal. Subsequent actions and
the original source/body/status/receipt/cleanup assertions remain active.

The fresh trace independently records actor 2's pre-START EOF terminal with
its main ended while maintainer `cd5149dd50e45e437f8a20ef` is still active,
running, MainPID 2533571, holding actual EX and lacking a terminal. This also
preserves the existing maintenance-2 poststate through controller loss.

The private driver adds only a successful-EOF diagnostic read of the actual
ordinary unit, exact retained terminal, and actual maintainer unit/phase/
fdinfo/terminal. It also corrects a stale comment about the old EOF-read
ordering. `final-driver.diff` has no behavior or projection change; the raw
seeded diff additionally contains the temporary input that was later removed.
No record is seeded or used as a substitute for an owner observation.

## Source and runtime attribution

Both receipts pin source HEAD
`1b623e6e167c4d3113efcf0d393c74748fbe9834` before and after execution,
including their actual unrelated dirty paths and private diagnostic diff.
That HEAD includes the approved P2 correction and the separately authored
guest-lane fixture propagation. The source production helper SHA-256 is
`f7d453b40d2c6523fa809daed8343d4e3648e7fbaa8e7aebc2c71cb7a98a5e8d`;
the unchanged sole model SHA-256 is
`f67ed362807560c981a60be33d127106f47329a290f5148add70fadcbd732b5e`.
The receipts also retain the exact driver and bootstrap hashes.

Both raw logs show ordinary checkout admission, source transfer, standalone
Git materialization, fail-closed native preflight, START, recompilation of the
selected xtask test binary from that transferred checkout, and terminal
aggregate 0. The existing host-pure conformance entry constructs real systemd
owners using a captured copy of the transferred production durable helper and
its real finalizer callback on the existing private fixture barrier. These
runs need no new guest preparation and make no claim about the contents or
behavior of a prepared guest image. Default-barrier public bootstrap and
guest effects remain their separately recorded acceptance boundaries.

## Local checks and limits

After restoring the default randomized input, scoped rustfmt and
`git diff --check` passed. Strict canonical Lima test-target Clippy passed:

```sh
cargo xtask lima run -- env CARGO_HOME=/tmp/metal-run-isolation-cargo CARGO_TARGET_DIR=/tmp/metal-run-isolation-target cargo clippy -p xtask --tests --features integration-tests --locked -- -D warnings
```

The raw idempotent Drop diagnostics for already-collected transient units
remain intact. No failure line, product diagnostic, or output difference was
normalized away. No broad suite, native guest campaign, direct SSH test,
mutation test, DES phase event, model change, or production change was made.
Other pending acceptance work retains its prior disposition.
