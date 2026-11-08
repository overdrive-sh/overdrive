# Ordinary concurrency native attempt 4 — overlap passed, survivor witness missed

This stable copy preserves the root-orchestrated capture from
`/tmp/metal-native-retry-20261108T221035Z`, including the raw controller
streams, every observation snapshot, the early polling failures, and
`orchestrator-observer-stop.json`. The original `/tmp` capture was not edited.

The existing `worktree_journey.sh` observed the two ordinary runs concurrently
and wrote `overlap.json` only after its complete overlap predicate passed. Its
later survivor-boundary predicate did not pass: the retained
`boundary-148.json` sample shows both durable invocations already terminal,
with no processes and no owned roots. The expected one-completed / one-live
survivor interval was not observed. The root orchestrator then stopped only
the local observer after both native invocations had completed; it did not
interrupt either native job. This is **not** a complete K1–K3 proof or a native
acceptance pass.

## Source and substrate attribution

Both commands used the existing explicit core guest-suite invocation:

```text
env OVERDRIVE_FWD_GUEST_GROUP=core cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests
```

The bootstrap transcripts identify target `ubuntu@151.115.99.251`. The guest
console output reports Linux `7.0.0-38-generic`. The capture does not persist
the full invocation environment or the configured kernel/rootfs paths, and
the read-only observer did not record the physical host kernel release.
`before.json` retains the physical baseline returned by that observer,
including BPF programs, module sizes, nftables, routes, test cgroups, and
initial vsock namespace modes. No `after.json` exists, so the required final
physical delta comparison was not made.

| Input | Source marker commit | Source digest | Checkout key | Captured status |
|---|---|---|---|---|
| `/Users/marcus/conductor/workspaces/helios/ouagadougou` | `254db62158d39f58e66a0afe1c74d6d2ec625870` | `fee1a0da6ce9d5d7e055cdfbed2348325016ccfcf90e598b6e1152ce8ffd1353` | `e3d831d7dafe621226ae6047ebcac3536f37470cafcf5dbb64eb9598886af410` | Dirty source paths are recorded in `first-source.status`, including preserved service-feature and review work. |
| `/private/tmp/metal-run-isolation-peer.UKMMtE6B` | `2ba2232208ce9a5e91666f61b6b9845d9797c62a` | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` | `052a646313948d416e08f7afb85c82411ecddc98659ed0825aa09bcde3be0f39` | `second-source.status` is empty. |

## Observed ordinary overlap

The driver registered `548afb1de9fd702b355bde44` for the current workspace and
`50c24c1fc933a91835db14e4` for the prepared peer. Both controller streams
contain `OVERDRIVE_METAL_RUN_STARTED`. In `overlap.json`, both owner records
are `ordinary`, active, and `running`. The driver predicate passed for the
shared-barrier read lock, distinct checkout locks, source marker bindings,
root ownership and modes, and the two live Cloud Hypervisor processes holding
`/dev/vhost-vsock`:

| Run | VMM PID | Network namespace | Existing CID |
|---|---:|---|---:|
| `548afb1de9fd702b355bde44` | `1816954` | `net:[4026533041]` | `16785416` |
| `50c24c1fc933a91835db14e4` | `1816955` | `net:[4026532936]` | `16785416` |

The repeated polling tracebacks in `driver.stderr` include failed intermediate
samples. They are not the final failure: the successful `overlap.json` is the
sample that satisfied the overlap predicate.

## Missed survivor boundary and stop attribution

`survivor-boundary.json` is absent. The driver's embedded survivor check at
`infra/metal/tests/worktree_journey.sh:93-108` requires exactly one completed
owner and exactly one live owner. In `boundary-148.json`, both runs have
`terminal.v1 kind=command exit_code=0 cause=command_returned`,
`completion.v1 kind=complete`, inactive units, zero observed processes, and all
three owned roots absent. Both terminal and completion timestamps are
`2026-11-08T22:12:33Z`. That sample therefore contains two completed owners and
no live survivor, so it cannot satisfy the driver's one-completed / one-live
assertion.

The root orchestration record says the local observer was stopped after both
native invocations were observed terminal 0 / complete with no processes or
owned roots, because the required survivor witness had been missed and could
not reappear for these terminal invocations. The record identifies local
driver PID `21341`. The orchestration wrapper reported `-15/241`; this was a
local observer stop, not the driver's normal no-survivor exit, and neither
native job was interrupted. The original raw capture does not contain a
normal driver exit status.

The two durable runs continued independently after START and both reached
their actual command result. The retained state does not demonstrate a
production cleanup defect or a completed survivor-preservation proof. It only
shows that the driver did not capture the required intermediate witness in
this run.

No `first-complete.json`, `after.json`, or final observer collection is
present. Therefore this attempt does not establish final physical-kernel
complement, the final two-run process/root complement, or an accepted K1–K3
result. The captured peer console reports `86 passed; 0 failed`; the current
workspace controller was stopped before its local output stream recorded the
same detail, although its retained terminal record is exit 0. No admission or
placement driver was run in this attempt, and no DES phase event was recorded
for this root-executed native attempt.
