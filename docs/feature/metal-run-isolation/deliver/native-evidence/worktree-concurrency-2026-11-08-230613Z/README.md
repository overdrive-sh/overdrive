# Concurrent ordinary worktree capture — 2026-11-08 23:06Z

This directory preserves the 64 raw files copied byte-for-byte from
`/tmp/metal-native-full-20261108T230613Z-worktree`. The root foreground run
reported driver exit 0 and emitted:

```text
METAL-ISOLATION-JOURNEY overlapping_ordinary_commands=true attributable_results=true physical_complement_unchanged=true owned_roots_absent=true
```

The capture directory has no standalone driver status or stdout file; the exit
and marker above are attributed to the root run report. `driver.stderr` retains
failed intermediate polling assertions, while `overlap.json` is the later
snapshot that satisfies the overlap checks. This evidence covers the ordinary
worktree journey only. It does not record admission or placement driver runs.

## Source and substrate

Both invocations used the existing ordinary guest-suite command on
`ubuntu@151.115.99.251`:

```text
cargo xtask metal run -- env OVERDRIVE_FWD_GUEST_GROUP=core cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests
```

The source identity below comes from each owner’s captured metadata and source
marker. The first source status is preserved in `first-source.status`; the
second source status is empty.

| Invocation | Workspace | Run ID | Commit | Source digest | Checkout key |
|---|---|---|---|---|---|
| First | `/Users/marcus/conductor/workspaces/helios/ouagadougou` | `b9a77315b8fe826db180f152` | `2b6a55b47c7412c8aaaefa58a9dc43f2eae3fdf1` | `cbcb61d6a5d4f9646770dfa4ef505ae87308abf59f8ac0004a60bce3877899c2` | `e3d831d7dafe621226ae6047ebcac3536f37470cafcf5dbb64eb9598886af410` |
| Second | `/private/tmp/metal-run-isolation-peer.UKMMtE6B` | `d35cb3e34528fb61bbf10ee5` | `2ba2232208ce9a5e91666f61b6b9845d9797c62a` | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` | `052a646313948d416e08f7afb85c82411ecddc98659ed0825aa09bcde3be0f39` |

The remote command transcripts identify the target host. `before.json` and
`after.json` preserve the observer’s physical baseline fields: BPF programs,
initial vsock modes, modules, nftables, routes, and test cgroups. These raw
files do not record a physical host kernel release.

## Overlap, survivor, and completion observations

`overlap.json` records both owners as ordinary, active, and running, with
matching source-marker digests and distinct checkout keys. Both had a live
Cloud Hypervisor process holding `/dev/vhost-vsock`, shared guest CID
`16785416`, and distinct network namespaces:

| Run | VMM PID | Network namespace |
|---|---:|---|
| `b9a77315b8fe826db180f152` | `1853456` | `net:[4026532989]` |
| `d35cb3e34528fb61bbf10ee5` | `1853457` | `net:[4026532936]` |

The driver intentionally stopped the first invocation’s supervisor. Both
first-run collection attempts returned status 70 with the retained
`supervisor_lost` error. `survivor-boundary.json` records the first owner
complete with no command terminal, no processes, and all three owned roots
absent. The second owner remained active with its checkout, source marker,
roots, VMM PID, and network namespace unchanged. Its retained guest-suite
output reports 86 passed and 0 failed; its command terminal reports exit 0.

`after.json` records both owners complete, with no live processes and all three
owned roots absent. The raw physical `before.json` and `after.json` snapshots
are equal. The driver’s comparison rule projects only paired nonnegative
route-expiry countdowns where both routes have protocol `ra`; in this capture
zero RA expiry values differed, so the projection changed no values. The raw
snapshots remain unmodified.

This is point-in-time evidence for the two captured source identities. It does
not establish the later preparation and guest-CWD changes or complete the
admission and placement journeys.
