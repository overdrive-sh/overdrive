# Bare-metal probe box

A disposable Scaleway Elastic Metal server for the microVM work that the Lima
dev VM **cannot** gate: Cloud Hypervisor runs nested on Apple Silicon there, so
boots stall ~2/3 of the time and `--memory shared=on` never boots at all. See
`docs/feature/microvm-driver-cloud-hypervisor/spike/findings.md`.

Bare metal removes the nesting. That is the whole reason this exists.

## Files

| File | Runs where | What |
|---|---|---|
| `cloud-init.yaml` | Scaleway, at install | The bare minimum to make the box **reachable**. Nothing else. |
| `partitions.json` | Scaleway, at install | Custom disk layout — no RAID, second disk left raw. |
| `bootstrap.sh` | Your laptop | rsync the tree up, then drive provisioning over ssh. |
| `provision.sh` | The box, as root | Media check, confinement identity, disks — then calls `../provision/*.sh`. |
| `../provision/*.sh` | The box | Provisioning. Written to be reusable by Lima, but **Lima does not invoke it today** — `infra/lima/overdrive-dev.yaml` still carries its own inline blocks, so the two drift. See the scope note in `common-system.sh` for the known divergences. |

## Order of operations

1. **Order the server.** Ubuntu. Paste `cloud-init.yaml` into the Cloud-init
   box. Choose *Custom configuration* for partitions and paste
   `partitions.json`. **Tick your SSH key** — Elastic Metal bakes in the keys
   present at install time; a key added afterwards does not appear.
2. **Wait for install**, then confirm by hand:
   ```bash
   ssh-keygen -R <ip>            # host key changes on reinstall
   ssh ubuntu@<ip> 'systemctl is-active ssh'
   ```
3. **Provision:**
   ```bash
   infra/metal/bootstrap.sh ubuntu@<ip> --data-disk /dev/sdb1
   ```
4. **Iterate** on probe code afterwards:
   ```bash
   infra/metal/bootstrap.sh ubuntu@<ip> --sync-only
   ```

## Things that cost time on 2026-08-10 — read before debugging

**The login user is `ubuntu`, not `root`.** Root's `authorized_keys` carries the
stock forced-command banner telling you so.

**Ubuntu's image installs `openssh-server` but may leave `ssh.service`
disabled.** The box then boots, configures its network, answers ICMP, and sends
TCP RST on :22 because nothing is listening. `cloud-init.yaml` enables it, which
is why that file exists and why it must stay minimal — an SSH-driven script
cannot fix a box you cannot SSH into.

**Check media before believing anything else.** A box shipped with failing
disks and the symptom was not obvious: it booted, networked, and accepted
logins, but individual files returned `Input/output error` while their
neighbours in the same directory read fine, and binaries took `Bus error` as the
page cache drained. Hours went into the partition layout, the SSH key, and the
provider's console before anyone read a raw sector. `provision.sh` now does a
head+tail read of every disk and refuses to continue on failure.

The tell: **failures that are per-file and 100% reproducible, on the same
filesystem, are bad sectors.** A partitioning error cannot be file-selective —
it breaks the whole filesystem or none of it.

**On bad media, request a replacement — do not reinstall.** A reinstall lays
the OS back onto the same failing sectors, and a corrupted install write is the
likely explanation for the missing `ssh.service` symlink in the first place.

**Scaleway's rescue mode uses the user `rescue`** and netboots a RAM disk, so
nothing provisioned there survives. It is for diagnosis only. `bootstrap.sh`
against rescue mode does nothing useful.

**KVM-over-IP may be unusable on HP hardware.** If the console floods with
`DMAR: [INTR-REMAP] ... Blocked an interrupt request`, check the device id — on
the box we had, `01:00.0` was the iLO **itself**, so opening the console
generated the interrupts that produced the spam that made the console
unreadable. Use ssh.

## Custom partitioning is OPTIONAL — prefer the default

The only reason to touch partitioning is to obtain a **reflink-capable
filesystem** for P4 (`cp --reflink=auto`), since ext4 has no `FICLONE` and
Scaleway's installer accepts only `ext4`/`fat32`. The default layout mirrors
BOTH disks into RAID1, leaving no free device.

**Two ways to get there. Prefer the second.**

1. `partitions.json` — custom layout, no RAID, second disk left raw.
   **Verified working on the EM-A116X-SSD (BIOS/SATA) only.**
2. **Default layout + `--break-raid-disk`** — take Scaleway's default
   untouched, then reclaim the second mirror leg after install:
   ```bash
   infra/metal/bootstrap.sh ubuntu@<ip> \
     --break-raid-disk /dev/nvme1n1 --data-disk /dev/nvme1n1
   ```
   Same end state, and it does not depend on the installer accepting a
   hand-written layout.

### Why (2) is safer — a trap worth knowing

Firmware mode differs per offer, and the partition layout MUST match:

| Offer | Firmware | First partition |
|---|---|---|
| EM-A116X-SSD (SATA) | **BIOS** | `"label": "legacy"`, no ESP |
| EM-I120E-NVMe (EPYC) | **UEFI** | `"label": "uefi"` + `fat32` at `/boot/efi` |

A layout copied from a BIOS machine onto a UEFI one **omits the ESP and will
not boot**. Always start from *that machine's own* default JSON (the console
shows it) and edit minimally — never reuse another offer's file.

### Device naming

SATA is `/dev/sda`, `/dev/sdb` with partitions `sda1`. NVMe is `/dev/nvme0n1`,
`/dev/nvme1n1` with partitions suffixed `p1`, `p2` — so `/dev/nvme1n1p1`, and
the whole disk is `/dev/nvme1n1`.

## Why the second disk is left raw

`[D5]` depends on the per-launch rootfs copy cost, and **ext4 has no `FICLONE`
support** — on ext4 the measurement silently degrades to a full copy and you
would wrongly conclude reflink does not help. Scaleway's installer only accepts
`ext4`/`fat32`, so the partition is left unformatted and `provision.sh` runs
`mkfs.xfs -m reflink=1` itself, then proves it with a real
`cp --reflink=always`.

If the custom partition JSON is ever rejected, `--break-raid-disk /dev/sdb`
reclaims the second mirror leg from Scaleway's default RAID1 after the fact.
