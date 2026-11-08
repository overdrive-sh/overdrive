# Ordinary concurrency native attempt 5 — exact host-route snapshot differs

This directory preserves the complete root-orchestrated capture copied from
`/tmp/metal-native-full-20261108T224051Z-worktree` before later source sync.
The corrected existing worktree driver completed its overlap, survivor,
collection, and final-observation stages, then exited 1 at the physical
test-kernel equality assertion. Its exact failure is real; this artifact does
not claim K3 or full native acceptance passed. Admission and placement drivers
were not run after this failure.

## Source and substrate attribution

The paired commands were the existing explicit `core` guest-suite invocation:

```text
cargo xtask metal run -- env OVERDRIVE_FWD_GUEST_GROUP=core cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests
```

The bootstrap transcripts identify target `ubuntu@151.115.99.251`. The guest
console reports Linux `7.0.0-38-generic`. The capture does not record the full
invocation environment or configured guest kernel/rootfs paths, and the host
observer does not record the physical host kernel release. `before.json`
preserves the physical initial-namespace baseline returned by the existing
observer: modules, nftables, BPF programs, routes, test cgroups, and vsock
modes.

| Input | Source marker commit | Source digest | Checkout key | Captured source status |
|---|---|---|---|---|
| `/Users/marcus/conductor/workspaces/helios/ouagadougou` | `0f6aa416c8abcdec7cdec7680906d2564d0e29ef` | `3fa166efee8283cb106ede9bad4a9fa7531cfc4eeeb61e2a8e09ec82b7e8c85f` | `e3d831d7dafe621226ae6047ebcac3536f37470cafcf5dbb64eb9598886af410` | See `first-source.status`; preserved unrelated service-feature and review work was present. |
| Prepared peer `/private/tmp/metal-run-isolation-peer.UKMMtE6B` | `2ba2232208ce9a5e91666f61b6b9845d9797c62a` | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` | `052a646313948d416e08f7afb85c82411ecddc98659ed0825aa09bcde3be0f39` | `second-source.status` is empty. |

The run IDs, in input order, are `3725880980a7280c2bb632a9` for the current
workspace and `367d69952aa7bf99dc8e13b6` for the peer.

## Owner and run observations

`overlap.json` was saved after all overlap assertions passed. Both records are
ordinary active owners in `running`, with distinct checkout locks, shared
barrier read locks, matching source markers, and their invocation-owned roots
at the expected ownership and mode. Each has a live Cloud Hypervisor VMM
holding `/dev/vhost-vsock`; both use CID `16785416` in distinct local network
namespaces:

| Run | VMM PID | Network namespace |
|---|---:|---|
| `3725880980a7280c2bb632a9` | `1846502` | `net:[4026532989]` |
| `367d69952aa7bf99dc8e13b6` | `1846303` | `net:[4026532936]` |

The corrected driver intentionally stops the first invocation's systemd owner
after disconnecting its controller (`worktree_journey.sh:84-95`).
`survivor-boundary.json` shows the first owner stopped with a complete cleanup
receipt, no terminal command record, no processes, and all three roots absent.
The second owner remained active with its checkout, source marker, roots, VMM
PID, and network namespace unchanged. The two first-run collections both
returned the retained `supervisor_lost` status 70. This is the designed honest
missing-terminal outcome: the driver did not promote it to a command result.
The peer collection/repeat returned command exit 0 and its guest log reports
86 passed, 0 failed. `after.json` records both units stopped, with no processes
or owned roots.

The driver then exited 1 at its direct equality assertion in
`infra/metal/tests/worktree_journey.sh:140`:

```python
assert before['physical'] == after['physical'], 'physical test-kernel complement changed'
```

## Exact physical snapshot difference

The recursive `before.json` / `after.json` comparison changes only two entries
in the host route list. Both routes remain present with the same route
identities and attributes; only their RA expiry timers differ:

| Host route on `eno12399np0` | Before `expires` | Overlap | Survivor boundary | First complete | After `expires` |
|---|---:|---:|---:|---:|---:|
| `2001:bc8:1f90:1f::/64`, `protocol=ra`, metric 100 | 2,591,997 | 2,591,999 | 2,591,996 | 2,591,998 | 2,591,998 |
| `default` via `fe80::2e4f:52ff:fefc:65d3`, `protocol=ra`, metric 100 | 12 | 14 | 11 | 13 | 13 |

No module, nftables, BPF program, test-cgroup, vsock-mode, or other route field
changed in the captured physical snapshots. The expiry values moved both up
and down between observations; that is consistent with sampling the lifetime
of the same router-advertised routes as it counts down and is refreshed. The
capture does not include the advertisement packet or identify the host process
that refreshed it, so that attribution remains an inference rather than a
directly observed event.

## Attribution against the accepted contract

The accepted feature contract is zero **test-owned** physical-host route
mutations, alongside modules, nftables, BPF and test cgroups
([feature-delta.md, scenarios and system constraints](../../../feature-delta.md#system-constraints)).
The external observer reads raw `ip -j route show table all` output at
`infra/metal/tests/observe_host.py:19-35`; it retains each `expires` value
without normalizing the timer. The driver compares the entire serialized
physical snapshot at line 140. That made this run fail on an observed
time-varying RA lifetime.

The actual selected command path does not provide an owner for a host initial
namespace route update. Ordinary explicit-suite commands pass through
`infra/metal/bootstrap.sh:788-796` and
`infra/metal/test-wrapper.sh:147-203`, which creates an invocation-local
network namespace before executing the command. The selected `run_core`
composition boots the existing disposable guest group and runs its tests
inside that guest (`crates/overdrive-testing/bin/fwd_guest_suite.rs:2033-2068`).
The observed VMM command lines in `overlap.json` include guest disk and vsock
launch arguments, but no `--net` device. There is no route-write operation in
this run's caller or owner path that can update the host's initial-namespace
RA routes.

The evidence therefore does **not** show a test-owned physical route
mutation, so it does not falsify P3's test-owned isolation premise. It does
show that the current whole-route-snapshot equality assertion treats a
volatile RA expiry observation as a forbidden delta. No assertion was
weakened here; the observed physical-delta check failed, so K3 and full native
acceptance remain uncredited pending an approved, attribution-preserving
evidence rule. The raw `after.json` is preserved and must not be rewritten to
make it match the baseline.

No admission or placement driver was run after this failure. No production,
test, model, or DES phase change was made during this diagnostic.
