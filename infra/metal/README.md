# Bare-metal KVM host

The shared native x86_64 KVM host (a Scaleway Elastic Metal server) on which
Overdrive's native-metal verification runs: the Cloud Hypervisor microVM
`kvm-tests`, the guest-kit lanes, and native-metal verification captures. The
Lima dev VM **cannot** gate that work: Cloud Hypervisor runs nested on Apple
Silicon there, so boots stall ~2/3 of the time and `--memory shared=on` never
boots at all. See `docs/feature/microvm-driver-cloud-hypervisor/spike/findings.md`.

Bare metal removes the nesting. That is the whole reason this exists. Several
worktrees use the box at once, each in its own checkout directory; day-to-day
use goes through `cargo xtask metal` (see "Durable runs and collection"), and
`bootstrap.sh` is invoked directly only to provision the box.

## Files

| File | Runs where | What |
|---|---|---|
| `cloud-init.yaml` | Scaleway, at install | The bare minimum to make the box **reachable**. Nothing else. |
| `partitions.json` | Scaleway, at install | Custom disk layout — no RAID, second disk left raw. |
| `bootstrap.sh` | Your laptop | rsync the tree up, then drive provisioning over ssh. |
| `provision.sh` | The box, as root | Media check, confinement identity, disks — then calls `../provision/*.sh`. |
| `../provision/*.sh` | The box | Provisioning, including `install-quint.sh` (pinned Node + Quint + JDK + Apalache under `/opt/quint-toolchain`, wrapper at `/usr/local/bin/quint`). Written to be reusable by Lima, but **Lima does not invoke it today** — `infra/lima/overdrive-dev.yaml` still carries its own inline blocks, so the two drift. See the scope note in `common-system.sh` for the known divergences. |

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
4. **Iterate** afterwards through `cargo xtask metal` (target from
   `OVERDRIVE_METAL_TARGET` in the environment or `.env`):
   ```bash
   cargo xtask metal sync
   ```
   `cargo xtask metal run` syncs as part of the run, so a separate `sync` is
   needed only to transfer source without running anything. Every sync also
   lands a standalone `.git` on the box (the repo's common dir plus this
   worktree's HEAD and index). The first sync moves the whole common dir; later
   syncs are incremental.

## Checkout-specific remote source

Each Git checkout owns one remote source and build directory. A linked
worktree's identity is its canonical worktree-specific Git directory, so a
branch switch or detached `HEAD` keeps the same remote directory. An ordinary
clone uses its physical checkout root and full branch ref; detached ordinary
clones are refused. The resulting full SHA-256 key selects
`~/overdrive-worktrees/<key>` or `~/overdrive-branches/<key>`. Branch spelling
is diagnostic only, so equal branches and names that sanitize alike cannot
merge source, build artifacts, or retained evidence.

Existing branch-name directories remain untouched. A new identity starts cold
and binds only a new or empty remote directory. The root file
`.overdrive-metal-checkout.v1` records the complete identity; an unbound
non-empty directory, malformed binding, or different owner is refused. An
`OVERDRIVE_METAL_REMOTE_SUBDIR` override selects a relative directory below the
physical remote home and receives the same full-owner check. The override is
refused if it contains an empty, `.` or `..` component, invalid characters, or
a symlink component.

Sync and pull share a lock derived from the canonical absolute remote
directory. Pull checks the binding but does not rebind it, so named evidence
can be retrieved after local source edits. `sync` and `pull` report a missing
remote `rsync` as `runner_preparation_failed` with status 75; they do not
install packages on the Metal box.

## Durable runs and collection

`cargo xtask metal run -- COMMAND [ARG ...]` registers a run record before
source transfer and native preflight, then starts the command under a root
transient systemd service. Ordinary runs take the shared physical-maintenance
barrier and the exclusive lock for their bound checkout. Independent worktrees
can run ordinary verification at the same time; sync, pull, and a run for the
same checkout still exclude one another. The service holds admission through
command completion and owned guest cleanup. It retains combined stdout/stderr,
the exact command result, and a completion receipt. The runner prints an
opaque 24-character run id:

```text
OVERDRIVE_METAL_RUN_ID=<run-id>
```

After `OVERDRIVE_METAL_RUN_STARTED` has been observed, losing the local SSH
route or the local `cargo xtask` process does not cancel the command. Collect
the retained log and exact command status later with:

```bash
cargo xtask metal collect <run-id>
```

Collection is read-only and can be repeated or run concurrently. It prints the
combined command log to stdout and emits a terminal marker on stderr. A command
status, including 255, is returned unchanged. Runner failures have stable
diagnostics for not-found, supervisor loss, malformed records, or a run that
never reached command start. Records live under
`/var/lib/overdrive-metal/runs/<run-id>/` on the box and are retained without
automatic pruning.

Ordinary commands use a closed existing-command grammar: Cargo
build/check/clippy/fmt, doctests, approved xtask build/setup/lint/model
commands, nextest, the prepared guest-suite executable, and the declared pure
Sim programs. Other commands refuse before execution. `cargo nextest run`
always uses the checked-in placement wrapper: host-pure tests keep the original
exec chain, native-launcher tests get a private node view (network namespace
in vsock `local` mode, a per-run cgroup namespace and cgroup2 mount, its own
bpffs, and private `/run/overdrive` and `/run/netns`, so a `serve` boot reclaims
only that run's state), and
guest tests run the selected Cargo-built test binary in a fresh instance of
its declared guest group. The wrapper preserves the selected binary, libtest
arguments, Nextest helper executables, test environment, retry number, and
actual attempt status. `guest-kit-prepare` prepares the dispatcher; the
wrapper does not build it on demand. The explicit guest-suite command keeps
its existing batched group behavior.

Use `--exclusive` for provisioning, physical-host maintenance, or another
existing command that needs arbitrary privileged authority. Never use it to run
tests: concurrent ordinary runs from different worktrees do not interfere. It takes exclusive
physical-maintenance ownership and waits for admitted ordinary runs to finish.
Native-metal verification captures (`verification/expectations/`) are not tests
and start from a runner script the ordinary grammar refuses, so they run
`--exclusive`, with their environment passed as `env KEY=VALUE` words (the local
environment does not reach the remote command).
For example, the host module-load step is an explicit administrative command:

```bash
cargo xtask metal run -- env OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build
cargo xtask metal run -- cargo xtask guest-kit-prepare
cargo xtask metal run --exclusive -- cargo xtask kmod-load
cargo xtask metal run -- cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests
```

The transient unit is intentionally not installed or reboot-persistent. A
reboot or an unrecordable supervisor/storage loss can leave a run without a
terminal record; collection reports supervisor loss and never invents a
command result. Generated files in the remote checkout still require an
explicit retrieval before a later one-way sync, which `cargo xtask metal pull`
performs.

### Placement, phases and configuration

`infra/metal/test-placement.json` maps every nextest binary id and test name to
the isolation `test-wrapper.sh` applies (`host-pure`, `native-launcher`,
`guest-group`, `guest-lane-operator`, `guest-lane-owner`). It is maintained by
hand and nothing checks it is complete: a test with no entry fails on its first
Metal run as `test_unclassified`, status 78. See `.claude/rules/testing.md`
§ "Test placement" for the kinds, diagnostics and authoring rules.

Each run record carries a phase: `waiting_for_lease` (queued for admission),
`preparing` (source transfer and native preflight), then `running`. Admission
waits up to `OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS` (default 120). Every run
also requires `OVERDRIVE_METAL_KERNEL` and `OVERDRIVE_METAL_ROOTFS` (readable
guest kernel and rootfs paths on the box); preflight fails closed with
`preflight_failed probe=guest_kernel` or `probe=guest_rootfs` without them.

## Pulling outputs back

```bash
cargo xtask metal pull specs/quint/<subsystem>/evidence
cargo xtask metal pull <path> [<path> ...]
```

Each named repo-relative path is rsynced from the box's tree bound to the
current Git checkout to the same path in the local checkout
(`bootstrap.sh --pull -- <path>...`). It takes that checkout's lock, so it
cannot race a sync or a run of that checkout, and it prints one
itemized line per transferred entry. It transfers only what is named, never
`target/`, and never deletes local files (no `--delete`), so files removed on
the box stay locally until you remove them. Absolute paths and paths with `..`,
`.`, empty or `.git` components are rejected, as is anything under `target/`.
A missing remote path or a nonzero rsync exit fails the command with that
status.

## Running the guest-kit `kvm-tests` lanes

The guest-kit lanes (the vsock walking skeleton, fault matrix and density
cells, the forwarder's VM-boundary cells, and the in-guest lane) do not build
anything inside a test. Run the setup commands above first against the same
checkout. The ordinary nextest command routes selected kernel tests through
their declared group, and retries start a fresh guest attempt. Do not run
`kmod-load` as part of an ordinary test command.

`guest-kit-prepare` builds `ovd_fwd.ko` for the running kernel, the static
scripted guest peer, the in-guest test binaries, and the host-side guest-suite
dispatcher, and stages each at the path its existing fixture reads
(`xtask/src/guest_kit_prepare.rs`). A missing
artefact makes the fixture refuse with `LaneNotPrepared`, naming the path.
It also requires the running kernel's build tree and `sparse`, installed by
`infra/metal/provision.sh`; sparse is built from the pinned upstream commit
that passes the kernel's `checker-valid.sh`, and `musl-tools` builds the
static guest peer. The explicit exclusive `kmod-load` step loads this
workspace's module on the physical host for existing native prerequisites;
guest-group test bodies load their own module copy in their disposable
microVM.

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
