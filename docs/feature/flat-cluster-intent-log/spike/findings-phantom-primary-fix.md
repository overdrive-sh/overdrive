# Phantom-primary committed-write-loss spike

## Verdict

**PROVEN for both epoch-crossing paths.** Cross-epoch state sync and commit-first `SwapEpoch` now
persist and install every successor epoch at `(view, log_view) = (0, 0)`, abandon predecessor-epoch
view-change standing, preserve the replicated log/frontiers, and derive timers from the successor's
view-0 role. Both deterministic regressions pass, all 878 `viewstamp-proto` tests pass, the three
model-based phantom replays conform without monitor findings, scripted and simulated Quint checks are
green, bounded Apalache verification exits 0 with the full `safety` invariant, and VOPR seed 313
completes 4,000 ticks without a regression.

## Root cause

- `viewstamp-proto/src/endpoint/state_sync.rs::install_sync` installed a verified successor membership
  without changing `view`, `durable_view`, `log_view`, `svc_target`, `svc_from`, or `view_change`.
  Member 1 could cross from E0/view 1 into E1 while retaining view 1 and immediately satisfy E1's
  round-robin primary predicate.
- `viewstamp-proto/src/endpoint/checkpoint.rs::durable_root_with_successor` persisted that stale
  predecessor-epoch view, so a restart could recover the phantom view.
- `viewstamp-proto/src/endpoint/mod.rs::submit_swap_epoch` had the same gap on the commit-first path. A
  real E0/view-1 primary could commit a demotion, install E1 while retaining view 1, and immediately
  lead E1 even though E1 had only begun in view 0.
- `viewstamp-proto/src/storage/session/mod.rs::submit_root` treated view numbers as globally monotone.
  That rejected the protocol-required E0/view 1 to E1/view 0 durable transition. Durable view identity
  is the lexicographic `(epoch, view)` pair: view is monotone only within an unchanged epoch.

This is the failure warned about in VR Revisited §8.3: accepting an old epoch's view number in the new
group can create two primaries.

## Change summary

```text
viewstamp-proto/src/endpoint/checkpoint.rs
  Cross-epoch SyncRepersist roots persist successor view 0 / log_view 0. A predecessor-view
  root queued behind an epoch-advancing root is normalized to successor view 0 and its obsolete
  correlation is abandoned when the epoch root lands.

viewstamp-proto/src/endpoint/mod.rs
  SwapEpoch roots persist successor view 0 / log_view 0. The landed-configuration adoption
  point resets live/durable/log view, clears predecessor SVC/DVC state, preserves the replicated
  log/frontiers, and re-arms timers for the successor membership's view-0 role.

viewstamp-proto/src/storage/session/mod.rs
  Durable root submission orders view identity by (epoch, view), admitting view 0 only when
  the epoch advances and retaining the no-view-rewind check inside one epoch.

spec/vr_sc.qnt
  Both syncFrom and commit-first install model the successor at view/log_view 0 with no
  predecessor SVC/DVC standing. Phantom runs assert agreement, no fail-stop, and absence of
  phantom primaryship. Completeness excludes down replicas, which cannot act as a primary.

scripts/env.sh
  Supplies Apalache with an 8 GiB JVM heap inside the 16 GiB Lima VM.
```

No public API was added or changed.

## Regression tests

`endpoint::tests::state_sync::a_cross_epoch_sync_cannot_carry_an_old_epoch_view_into_primaryship`
makes retained member 1 the E0/view-1 primary, performs a verified E0 to E1 checkpoint sync, and
asserts the live state and durable root both carry view/log-view 0, no old DVC collection remains,
and member 1 is an E1 backup.

`endpoint::tests::reconfigure::a_commit_first_swap_cannot_carry_an_old_epoch_view_into_primaryship`
drives the real proposal, WAL append, successor-quorum acknowledgement, commit, durable SwapEpoch
root, and install path. Member 1 begins as the E0/view-1 primary and demotes member 2; the test proves
the E1 root and live endpoint are view/log-view 0, member 1 is an E1 backup, and the committed
reconfiguration head/frontier are unchanged.

### RED

State-sync regression:

```text
assertion `left == right` failed: a new epoch starts at view 0 instead of retaining the old epoch's view 1
  left: View(1)
 right: View(0)
Summary: 1 test run: 0 passed, 1 failed
```

Captured in `out/red-viewstamp-phantom-primary.log`.

Commit-first regression:

```text
assertion `left == right` failed: the successor epoch starts at view 0 instead of retaining E0/view 1
  left: View(1)
 right: View(0)
Summary: 1 test run: 0 passed, 1 failed
```

Captured in `out/red-viewstamp-commit-first-view-reset.log`.

The original pre-fix MBT replay independently captured both production failures:

```text
IMPLEMENTATION FINDING: [CatchUpView] FAIL-STOP: n2 panicked: must not rewind below our committed op
IMPLEMENTATION FINDING: [DeliverPrepareOk] AGREEMENT: n2 applied label 3 at op 2, but label 1 was committed there
client replies (client, request, label): [(1000001, 1, 1), (1000003, 1, 3)]
```

Captured in `out/red-phantom-primary.log`.

### GREEN

```text
test ...a_cross_epoch_sync_cannot_carry_an_old_epoch_view_into_primaryship ... ok
test ...a_commit_first_swap_cannot_carry_an_old_epoch_view_into_primaryship ... ok
test ...a_view_change_entered_inside_the_swap_window_carries_the_successor_configuration_forward ... ok
Summary: 3 tests run: 3 passed, 0 failed
```

Captured in `out/green-viewstamp-epoch-view-reset-window-2.log`.

Full Viewstamp protocol suite:

```text
Summary [10.404s] 878 tests run: 878 passed, 0 skipped
```

Captured in `out/green-viewstamp-proto-suite-after-commit-first-3.log`.

Fixed model-based replay:

```text
phantomFailStop: replay result = CONFORMS (every step matched); implementation monitors: no violation
phantomLoss: replay result = CONFORMS (every step matched); implementation monitors: no violation
phantomPrefix: replay result = CONFORMS (every step matched); implementation monitors: no violation
Summary: 3 tests run: 3 passed
```

Captured in `out/green-phantom-fixed-after-commit-first.log`.

## Quint and Apalache

- `quint test`: `phantomPrefix`, `phantomFailStop`, and `phantomLoss` all passed their safety
  assertions. Evidence: `out/green-quint-phantom-runs-after-commit-first.log`.
- `quint run`: 500 traces × 30 steps with seed `0x5eed` found no `safety` violation for both
  `vr_sc_4v` and `vr_sc_3v1l` after the commit-first model update. Evidence:
  `out/green-quint-safety-sim-after-commit-first-2.log` and the corresponding ITF files.
- `quint verify`: the Lima VM has 16 GiB RAM with about 13 GiB available at rest. Apalache's launcher
  defaulted to a 4 GiB heap and exhausted it, so `scripts/env.sh` now supplies `JVM_ARGS=-Xmx8g`, leaving
  half the VM for Z3, Quint, and the OS. The variable `1.to(cp)` range was also expressed as a filter
  over the constant `0.to(MAX_LOG)` domain so Apalache could translate it. This exact command checked
  the composite `safety` invariant to depth 2 and exited 0:

  ```text
  JVM_ARGS=-Xmx8g quint verify spec/vr_sc.qnt \
    --main=vr_sc_4v --invariant=safety --max-steps=2 --verbosity=2
  PASS #13: BoundedChecker
  State 0: Checking 10 state invariants ... all hold
  State 1: checked enabled successor states ... all hold
  exit=0
  ```

  Exact command and full Apalache pass log: `out/green-quint-verify-safety-final.log`. Its working
  directory was under `/var/tmp` and was removed after the private checker server stopped.

## VOPR

No VOPR regression was observed after the commit-first change.

```text
vopr seed 313 OK: ticks=4000 replicas=4 clients=4 max_committed=1088
crashes=6 restarts=6 partitions=2 heals=2 max_view=2
Summary: 1 test run: 1 passed
```

Captured in `out/green-vopr-seed-313-after-commit-first.log`.

## Remaining limits

- The bounded Apalache run is depth 2; the scripted phantom traces and 30-step randomized simulations
  provide the deeper crossing schedules.
- VOPR evidence is one documented 4,000-tick replay seed, as requested for the short regression sweep;
  this spike did not run the full multi-seed release matrix.
