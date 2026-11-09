# Independent DISTILL review — hook ownership conformance driver sequencing

## Metadata and verdict

| Item | Value |
|---|---|
| Scope | Bounded test-support sequencing change in `crates/overdrive-control-plane/src/worker/quint_hook_ownership.rs` |
| Author record | `hook-ownership-remediation.md` and retained `.context/step-01-02-distill-*` Metal logs |
| Context | Original DELIVER step 01-02 crafter owns its activated tests, dirty D3/B2 production edits, DES phases and commit |
| Verdict | **APPROVED** — the driver now paces one real observer event per model `Consume`, preserving its fault and production-owner oracle |

## Reproduced failure and need for the support fix

The original `QUINT_SEED=0x1a5311b9` replay failed at trace step 20, `Consume(2, false)`: the Quint state expected allocation 2's terminal row to be absent, while the production observation projection read it as present. The retained pre-fix log records the seed and invariant failure (`.context/step-01-02-distill-seed-before.log`); the detailed crafter replay records the expected/actual `row: false`/`row: true` difference (`.context/step-01-02-randomized-replay.log:5960–5987`). This was not evidence that the exit observer ignored a non-retryable write fault: the older driver's precheck drove its runtime before fault injection, and its wait for any prior `ReleaseSupervision(id)` could be satisfied by a shim release. The stale allocation 1 event consumed the fault meant for allocation 2.

The intermediate count-only wait exposed the second scheduling error at `QUINT_SEED=0x94ef88cb`: a runtime drive for `Consume(1, true)` drained allocation 2's already forwarded event as well. The retained verbose log shows the model's allocation 2 still queued (`sup: true`, `row: false`) while the projection had already consumed its event (`sup: false`, `row: true`) at `.context/step-01-02-distill-seed94-verbose.log:18877–18899`. These two concrete failures justify bounding the driver's delivery to one event per `Consume`; they do not justify a production behavior or DESIGN change.

## Contract and production-boundary audit

The Quint model's `Emit(a)` changes only its ghost `ev` state from `none` to `queued`; `Consume(a, wrote)` changes it to `consumed` and expects the row outcome (`hook_ownership.qnt:140–162`). The conformance projection compares only `ProbeRunner` supervisor presence and the terminal `AllocStatusRow` read through `ObservationStore::alloc_status_row` (`quint_hook_ownership.rs:52–61,915–965`). It does not read the driver's pending map, call count or fault setting as projected state. The separate S-4 acceptance bodies remain the proof of register-before-gate call ordering.

The revised `Emit` checks the same guest-death and released-gate preconditions, then queues one `ExitEvent` by allocation ID in private test support (`quint_hook_ownership.rs:715–737`). `Consume` removes only that allocation's event, captures the existing `ReleaseSupervision(id)` count, scripts the same non-retryable `ObservationStoreError::Io(PermissionDenied)` for `wrote = false` when no terminal row already exists, forwards that event through the existing `exit_tx`, and drives the observer until a **new** release call is recorded (`:739–767`). The per-ID queue prevents another model-queued event from taking the fault or being consumed during this `Consume`. It does not create a terminal row, call the hook, remove a supervisor, or decide the outcome itself.

The actual owner path remains `exit_observer::spawn_with_runtime` receiving the event on `rx.recv()`, calling `Driver::on_alloc_terminal`, running `run_with_retry` over the real observation port, and calling `release_supervision` after `Wrote`, `NoWrite` or `Failed` (`exit_observer.rs:204–323`). `PermissionDenied` is non-retryable under `ObservationStoreError::is_retryable` (`overdrive-core/src/traits/observation_store.rs:98–111,157–162`). Waiting for a count increase therefore observes completion of the **new observer event**, including its failed-write path. This is a test-support scheduling boundary around an unchanged production owner, not a driver-side substitute for that owner.

The private `pending_exits` map and `call_count` helper add no public API, Sim adapter seam, persisted state or extra model action. The diff changes `Emit`/`Consume` driver scripting and comments. The seven removed `#[ignore]` markers in the same file are the original crafter's step 01-02 activation and are preserved; this DISTILL reviewer made no change to them. The Quint spec, conformance projection, fault type, row/supervisor oracles, source adapter and temporary seed configuration were not modified. `.cargo/config.toml` and `crates/overdrive-sim/src/adapters/observation_store.rs` are clean.

## Verification and disposition

| Evidence | Observed result |
|---|---|
| `QUINT_SEED=0x1a5311b9` before | 1 failed randomized run; false-terminal-row divergence at `Consume(2, false)` as above. |
| `QUINT_SEED=0x94ef88cb` with count-only intermediate | 1 failed randomized run; a second queued event was consumed during `Consume(1, true)`. |
| Both seeds with final private queue and fresh-release wait | Each randomized replay passed (`.context/step-01-02-distill-seed1a-fixed.log`, `step-01-02-distill-seed94-fixed.log`). |
| Final hook conformance selection | 7/7 passed through `cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --features integration-tests --lib --no-fail-fast -E 'test(quint_hook_ownership)'` (`.context/step-01-02-distill-hooks-final.log`). |
| Static checks | Author's scoped `rustfmt --check`, `git diff --check` and Metal `cargo clippy -p overdrive-control-plane --lib --features integration-tests -- -D warnings` passed (`hook-ownership-remediation.md`; retained Clippy log ends with exit code 0). My read-only `git diff --check` passed; I did not rerun cargo or mutation. |

**APPROVED.** The original step 01-02 crafter may adopt this bounded DISTILL test-support correction while retaining ownership of its production implementation, activated test markers, DES log and commit. This review approves only the conformance-driver sequencing and does not claim the step's production code is reviewed or complete.
