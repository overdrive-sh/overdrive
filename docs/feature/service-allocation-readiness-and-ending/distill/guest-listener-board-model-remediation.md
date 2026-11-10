# Step 05-02 DISTILL remediation: guest listener board and host-report model

## S-50 board property: first report versus duplicate

The active board property failed on Metal before this edit with the shrunk case `steps = [Duplicate(0)]`, `after_close = []`. Its generated `Duplicate(0)` was the **first** report of TCP 8080. The production beacon reader correctly inserted that declared pair, while the test simultaneously expected the set to contain it and to equal the previously empty set. D13 item 10 requires first valid insertion and makes only a *real duplicate* a no-op; no production defect was shown by this counterexample.

The test step is now called `Repeat` because an attempted repeat can be the first arrival. The oracle tests the no-op property only when the pair was already in the expected set before that step. Every generated case also drives an undeclared first line (empty-set no-op), one valid first insertion and a second delivery of that same pair (actual duplicate no-op) before the random schedule. The random schedule retains declared/undeclared reports, both allocation identities, monotonic set and declared-universe assertions, and the after-close and other-allocation complements. No board implementation or public surface changed.

## Host-report conformance: VMM exit closes the beacon connection

The randomized `guest_report_host` conformance failed reproducibly with `QUINT_SEED=0xd1acbdab` (`.context/step-05-02-guest-report-seed-before.log`). In the failing trace, `VmmExit` occurred after the reader had consumed an `EXIT` line while `eof=false`; the model then chose `AppendListen(udp53)` because its only append guard was `not(eof)`. The production handler maps `VmmExit` to `SimVmm::exit_process`, exit-gate release and the resulting exit-observer row. By the time that step returns, the guest beacon connection is gone: the next `SimBeaconIo::guest_write` returns false, and the driver rejects the action as not enabled. `NEXTEST` reported exactly that action-boundary refusal, not a mismatch in the projected reported set or exit reason.

The `guest_report_host` model now permits `HostRead`, `AppendListen`, `AppendExit`, `HostGarbage` and `BeaconClose` only while `exitrep == "none"`, the existing model phase before `VmmExit`. This uses no new state or action. It still permits a late `LISTEN` after the host reader consumes `EXIT` **before** VMM exit, while the channel remains open. The named `host_listen_after_exit_is_ignored` run still exercises that separate case and retains its unchanged reported-set and `guest-reported` exit assertions. The compared production projections (`rset`, `exitrep`), driver calls, full-module guest-side model, and approved invariants were unchanged.

## Verification and handoff

| Check | Result |
| --- | --- |
| Before: Metal S-50 property | Failed on `[Duplicate(0)]`: first valid insertion contradicted the no-op oracle. |
| After: all six `guest_listener_board` acceptance bodies on Metal | 6/6 passed. |
| Before: Metal conformance random traces, seed `0xd1acbdab` | Failed after `VmmExit` on `AppendListen(udp53)`: production guest write refused the closed connection. |
| After: same seed, then four named host-report runs plus unseeded random trace | Seeded 1/1 and suite 5/5 passed; the late-LISTEN-before-VMM-exit named run remained active. |
| Formal `cargo xtask quint check --subsystem service-allocation-readiness-and-ending --record` on Metal | 84/84 checks passed; refreshed evidence has 13 input hashes. Pulled and verified with `cargo xtask quint verify-evidence` (all outcomes met `expect`). |
| Changed Rust file rustfmt and `git diff --check` | Passed. |
| Strict worker Clippy attempted on Metal | Blocked by an in-flight production lint in `src/guest_listener_board.rs:55` (`significant_drop_tightening`), outside this DISTILL ownership. The acceptance test compiled and passed. |

The temporary `.cargo/config.toml` seed override was restored byte-for-byte after each seeded run; it is clean. Changed files are the S-50 test body, `guest_report.qnt`, this record, and the refreshed formal evidence under `specs/quint/service-allocation-readiness-and-ending/evidence/`. No production code, DES event or commit was changed here. Independent DISTILL review is required before the original step 05-02 crafter resumes GREEN and COMMIT.
