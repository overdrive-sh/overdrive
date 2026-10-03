# Spike A, increment a: Nanos under Cloud Hypervisor (PVH boot + vsock)

Throwaway probe for GH #303 (`in-guest-kernel-mtls`). Tests one assumption:
a Nanos unikernel boots under Cloud Hypervisor via Nanos's PVH entry, and
Nanos's virtio-vsock reaches the host.

Written before the boot run (debugging.md § 4):

- **Hypothesis:** a Nanos image boots under Cloud Hypervisor through Nanos's
  PVH entry, and Nanos's virtio-vsock reaches the host.
- **Prediction:** the guest boots; virtio-blk (root image), virtio-net and
  vsock come up (CH event monitor reports `virtio-device activated` for each);
  a guest `AF_VSOCK` connect to CID 2 port 5000 lands on CH's host Unix socket
  `<vsock socket>_5000`; the host receives the exact REQUEST bytes and the
  guest reads back the exact, different RESPONSE bytes. Static reads predict
  no `VIRTIO_VSOCK_F_STREAM` failure, because Nanos's `attach_vtpci` masks
  `dev_features & feature_mask` and never checks that requested bits survived
  (`nanos:src/virtio/virtio_pci.c:444`).
- **Falsification:** no boot (nothing on the serial console, or a halt before
  the program starts); or no vsock activation event / no connection on
  `<socket>_5000`; including Nanos refusing the device because
  `VIRTIO_VSOCK_F_STREAM` is not offered.

### Pass 2 (written after run 0003, before run 0006)

Run 0003 (unmodified `aad473aa`) died at PVH entry: CH log
`Guest MMIO write to unregistered address 0xfffffffc` → read of the same →
`VcpuRun(... InternalError)`. Mechanism: `pvh_start32` calls
`pvh_zero_page` (`init.s:116`) before `mov esp, 0xa000` (`init.s:142`); CH
v53.0 enters PVH with RSP=0 (`regs.rs:100-104`), so the call pushes to
`0xfffffffc`.

- **Hypothesis:** the ESP-before-first-call ordering is the *only* PVH-boot
  blocker; with `patches/0001` the rest of the chain works.
- **Prediction (run 0006, kernel-pvhfix):** serial shows Nanos output, the
  `NET: static IP config` lines and the `IGKM-A:` markers; the event monitor
  shows `virtio-device activated` for `_disk0`, `_net1`, `_vsock2`; both legs
  PASS on the host and in the guest. (run 0007, +diagnostic 0002) the vsock
  attach line shows `dev_features` without bit 0 (`VIRTIO_VSOCK_F_STREAM`)
  and negotiated `features` = `VIRTIO_F_VERSION_1` only (`0x100000000`).
- **Falsification:** a different early crash (another stack/ABI assumption),
  no RSDP/PCI discovery, a device that never activates, or vsock refusing
  to attach without `F_STREAM`.

## Files

| File | Role |
|---|---|
| `capture.sh` | Append-only evidence capture for one `cargo xtask metal run` → `runs/NNNN.{meta,stdout,stderr}` (refuses overwrite; redacts host). |
| `rsync-without-local-env.sh` | Keeps the workspace `.env` (SSH target) off the host during sync. |
| `host-inventory.sh` | Substrate probes (testing.md) + tool inventory. Creates nothing. |
| `build.sh` | User-space build on the host (no sudo, no packages): nasm 2.16.03 from source, Nanos `aad473aa` kernel/boot/mkfs, static guest, Nanos disk image. Under `~/igkm-spike-a` (outside the rsynced tree). |
| `guest/vsock_probe.c` | Static guest: boot marker, `AF_VSOCK` → CID 2:5000 exchange, `AF_INET` → tap exchange. |
| `guest/vsock_probe.manifest.in` | Nanos manifest (static IP, `log_level:info`). |
| `host_listener.py` | Host side: Unix listener on `<vsock base>_5000`, TCP listener on the tap IP. |
| `run-boot.sh` | Boots under CH (root), prints all logs, cleans up and diffs host state. |

## Reproduce

```bash
bash spike-scratch/in-guest-kernel-mtls/increment-a/capture.sh 0001 inventory -- \
  bash spike-scratch/in-guest-kernel-mtls/increment-a/host-inventory.sh
bash spike-scratch/in-guest-kernel-mtls/increment-a/capture.sh 0002 build --no-sudo -- \
  bash spike-scratch/in-guest-kernel-mtls/increment-a/build.sh
OVERDRIVE_METAL_KERNEL=/home/<user>/igkm-spike-a/nanos/output/platform/pc/bin/kernel.img \
OVERDRIVE_METAL_ROOTFS=/home/<user>/igkm-spike-a/out/nanos-vsock.img \
bash spike-scratch/in-guest-kernel-mtls/increment-a/capture.sh 0003 boot -- \
  bash spike-scratch/in-guest-kernel-mtls/increment-a/run-boot.sh
```

Findings: `docs/feature/in-guest-kernel-mtls/spike/findings.md`.
