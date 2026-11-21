# Node-local persistent volume

A VM Service that keeps data on a node-local volume (GH #348). The Service
declares `[vm.volume] size = "1GiB"`; the volume belongs to the workload, so it
is still there, with its data, after a restart, a stop and resume, and a node
reboot.

**Status.** This example deploys successfully only once the volume launch path
is delivered. Until then `overdrive deploy` rejects `[vm.volume]` or the
workload fails to start, and nothing here has been run. What it demonstrates is
the intended operator journey: declare a volume, see it in `describe`, restart
the workload, and read back the data the guest wrote earlier.

## What is here

| File | Purpose |
|---|---|
| `volume-service.toml` | A single-replica VM **Service** with a 1GiB volume. A Service, not a Job: a Job gets no successor when it ends, so its volume would never be reattached. |
| `volume-oversized.toml` | The same Service shape with a `1000TiB` volume, larger than any pool, to show the typed insufficient-space reason in `describe`. |
| `guest_volume.rs` | The guest program (std only, built static). |
| `prepare.sh` | Runs `../service-kind-vm-workloads/prepare.sh` (kernel, private rootfs, serve data directory, credentials) and installs the guest program into that rootfs. |
| `run-example.sh` | The journeys below, plus `prepare`, `serve` and `cleanup` as separate steps. |

Both specs name the kernel and rootfs that `prepare.sh` materialises under
`/srv/vm/overdrive-testing/svm-e08`. That is the service-kind VM bundle's
location, so this example and that bundle cannot be prepared at the same time.

## The guest

On every start the guest finds its second disk (virtio serial `ovd-volume`),
reads the notes earlier starts wrote, and appends one note. A note is one
512-byte sector of text:

```
OVD-VOLUME-NOTE seq=2 prior_verified=1 uptime_s=...
```

`seq` counts starts; `prior_verified` is how many earlier notes the guest read
back, in order, before writing this one. The guest then serves HTTP on port
18080: `GET /ready` answers 204 only if every earlier note read back intact and
the new one was flushed (503 otherwise), and `GET /records` returns the
read-back. The Service's readiness probe is `GET /ready`, so a Service that
`describe` shows ready is one whose guest verified its data. The volume holds
no filesystem: the notes are plain text at fixed offsets, so the host can read
the same bytes from the volume's device node.

## Prerequisites

A KVM-capable x86_64 node with the volume pool (the partition labelled
`overdrive-volumes`, and the `overdrive` volume group on it), a kernel and
rootfs at `/srv/vm/overdrive-testing/kernel` and `rootfs.ext4`, and a built
default-feature `overdrive` binary. `run-example.sh` refuses to start on a node
without the volume group. A pool is permanent: each distinct workload id keeps
its volume's space even after the workload is stopped.

## Run it

Through the native-metal runner:

```bash
cargo xtask metal run -- examples/node-local-volume/run-example.sh run restart-survives
cargo xtask metal run -- examples/node-local-volume/run-example.sh run oversized
```

`restart-survives` does, in order, with the public CLI only:

```bash
overdrive deploy --detach examples/node-local-volume/volume-service.toml
overdrive workload describe volume-notes     # Volume  1GiB (guest disk: virtio serial ovd-volume)
overdrive workload restart volume-notes      # a replacement attaches the same volume
overdrive job stop volume-notes              # the volume stays
overdrive workload restart volume-notes      # resume: the same volume is attached again
```

After each stop it reads the volume's device node from the host with direct
I/O, while no VMM holds it, and checks that the notes are all there:
`seq=1 prior_verified=0`, then `seq=2 prior_verified=1`, then
`seq=3 prior_verified=2`. It also checks that the pool gained exactly one
logical volume, however many times the workload started.

`oversized` deploys `volume-oversized.toml` and polls `workload describe
volume-oversized` until the allocation reason names the shortage. The reason
is a typed volume failure, not a generic error. The runner checks that the pool
is unchanged afterwards; the exact wording is whatever `describe` prints.

## Across a node reboot

The runner cannot reboot the node, so the reboot journey is the same commands
in separate steps. Rebooting a shared host needs its owner's approval.

```bash
examples/node-local-volume/run-example.sh prepare
examples/node-local-volume/run-example.sh serve &           # keep it running
overdrive deploy --detach examples/node-local-volume/volume-service.toml
overdrive workload describe volume-notes
# reboot the node; once it is back, with the preparation directory still present:
examples/node-local-volume/run-example.sh serve &
overdrive workload describe volume-notes
```

Both `overdrive` invocations need `OVERDRIVE_CONFIG_DIR=/srv/vm/overdrive-testing/svm-e08/config`.
After a boot the volume is inactive; the Service's next launch activates it,
with no operator step, and the guest reads the earlier notes back. The new
note's `prior_verified` shows how many. `run-example.sh cleanup` removes the
preparation directory; it never removes a volume.

## Limits

- A workload declares at most one volume (multiple volumes: GH #364), and a
  workload with a volume must declare `replicas = 1`.
- No operator command deletes a volume, so a stopped workload keeps its space
  (workload deletion: GH #211).
