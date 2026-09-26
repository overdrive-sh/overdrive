# ADR-0144 — Managed host links are protected from udev's MAC policy: the bridge is created with its address, and an appliance-shipped `.link` file exempts managed links

## Status

**Accepted (2026-09-26).** GH #295, platform requirement REQ-295-LINKMAC,
from root cause A of the fresh-host RCA
(`docs/analysis/root-cause-analysis-netns295-fresh-bridge-boot-refusal.md`).
It rests on two user rulings:

- **The appliance-OS ruling of 2026-09-25** (feature delta, § *User ruling of
  2026-09-25*). In the user's words: "This will be running on our own
  appliance OS with preinstalled dependencies." ADR-0068 records that Overdrive
  ships its own appliance OS and controls what it boots. Together they make the
  host's link configuration Overdrive's to ship. That conclusion is this ADR's
  reading of the ruling, not the ruling's own words.
- **The user's ruling that technical decisions for #295 are settled on
  evidence.** The DESIGN pins of 2026-09-25 and 2026-09-26 were made under it.
  The evidence here is the RCA's probes on the Lima VM (kernel `7.0.0-31`,
  systemd 259) and the upstream kernel v7.0 and systemd v259 sources it cites.

It depends on:

- ADR-0126, the fixed bridge MAC;
- ADR-0130, the audit read-back of each TAP's host-side MAC;
- ADR-0143, which denies Cloud Hypervisor `SIOCSIFHWADDR`;
- ADR-0068, which makes the appliance image Overdrive's to build.

The exact contract lives only in the #295 feature delta
(`docs/feature/netns-density-295/feature-delta.md`, § *Managed-link address
from creation, and the host link-address policy*). That contract covers the
bridge-creation operation, the file's name, content, and install locations,
the installation check, and the startup-probe condition.

## Context

The #295 design gives every managed host link's address one writer. The
managed links are the node bridge `ovd-gbr0`, the startup probe's scratch
bridge `ovd-gbr-probe`, every guest TAP `ovd-tp-<4hex>` (named by ADR-0118),
and the probe's scratch TAP `ovd-tp-probe`.

- The shared guest-network owner converges the bridge to the fixed
  `02:01:00:00:00:01` and reads it back. A mismatch at boot refuses startup. At
  runtime the audit repairs it under quiescence (ADR-0126).
- A TAP keeps the address the kernel assigns when the TAP is created.
  Provision records it, and activation and every audit compare against it. A
  change is per-allocation damage, and only that VM is killed (ADR-0130,
  ADR-0124). Cloud Hypervisor cannot change it (ADR-0143).

The host runs a second writer that the design did not account for.
systemd-udevd applies the first matching `.link` file to every new link. The
default `99-default.link` matches every name, with `MACAddressPolicy=persistent`.
When udev processes a link's add event, it reads `addr_assign_type`:

- if the kernel assigned the address at random, udev replaces it with a MAC
  derived from the machine and the link name;
- if userspace set the address, udev's address policy leaves it alone (systemd
  v259 `src/udev/net/link-config.c:605-606`).

The fresh-host RCA traced a boot refusal to this writer:

- The owner creates `ovd-gbr0` without an address and then sets it. The kernel
  registers the bridge with a random address and emits its add event inside
  `register_netdevice`, before the creating `RTM_NEWLINK` is acknowledged. So
  udev can read "random" before the owner's set marks the address as set by
  userspace.
- On the Lima VM, udev's write landed 0.19 ms after the owner's set, and the
  boot read-back refused. This happened in 6 of the 9 fresh-host boots that
  reached the check, and in 3 of 6 on an earlier tree. The defect has been
  latent since the read-back landed. A boot that adopts an existing bridge
  gets no add event and passes, which is why a retry appears to fix it.
- A bridge created with its address in the same `RTM_NEWLINK` is marked as set
  by userspace before registration (`net/core/rtnetlink.c:3698-3701`), so its
  add event never shows a random address. The RCA read it that way at the first
  observation in 10 of 10 trials, and every one kept its address. Adding and
  removing ports does not change such a bridge's address
  (`net/bridge/br_stp_if.c:263-265`).

TAPs cannot be created that way. The tun driver assigns a random address at
creation and refuses creation over rtnetlink (`drivers/net/tun.c:1332`,
`:2283`). A TAP's add event therefore always shows a random address, and udev
rewrote the probe's scratch TAP 3.6 ms after creation. The RCA predicts that
once provision records the host-side MAC, a rewrite that lands after the
record reads as damage, and the audit kills a healthy VM. That prediction is
not yet reachable, because nothing records the address yet. Whether it is
demonstrated, with a control host that lacks the file, is DISTILL's decision.

With a `.link` file matching `ovd-*` and setting `MACAddressPolicy=none`, udev
named that file as applied, and a new TAP kept its kernel address and random
assign type after 1 s. A control TAP in the same session, under the default
policy, was rewritten.

A repair loop cannot close this race, because the race happens at creation:

- udev writes asynchronously, a few to tens of milliseconds after the add
  event (3.6 ms at info level and 28–31 ms at debug level in the RCA), and it
  gives the creator no completion signal the platform reads. A read-back can
  pass before udev writes, and any write the owner makes after the add event
  is itself inside the window. Re-reading until stable, or waiting a settle
  delay, is a timing heuristic that nothing the owner controls bounds.
- The only deterministic completion signal is udev's own device database.
  Reading it at runtime would add a udev library dependency, or a `udevadm`
  shell-out of the kind ADR-0085 removed.
- For a TAP, a repair would erase the detection it depends on. The audit's
  host-side-MAC read-back exists to catch a changed address as tampering
  (ADR-0130, ADR-0142). Re-converging the address would make a host daemon's
  write and a compromised process's write look the same.
- The runtime audit already repairs the bridge MAC, but only after closing
  EXEC and quiescing every managed TAP (ADR-0126, ADR-0124). Leaving a
  creation-time write for the audit would turn every fresh boot on a default
  host into a recovery.

## Decision

A managed host link's address is written only by `overdrive serve`, or by the
kernel at creation. No host link manager writes it.

- **The bridge is created with its address.** The shared owner creates the
  node bridge, and the startup probe creates its scratch bridge, with the fixed
  address carried in the same `RTM_NEWLINK` that creates the link,
  administratively down. udev's address policy therefore never rewrites it,
  whatever policy the host sets. A bridge that already exists is adopted
  without a write and then converged, as ADR-0126 specifies.
- **Managed links are exempt from udev's MAC policy.** A host that runs
  systemd-udevd carries one Overdrive `.link` file. It matches the managed
  bridge and TAP name prefixes, `ovd-gbr*` and `ovd-tp-*`, and sets only
  `MACAddressPolicy=none`. It sorts before the default and netplan-generated
  files, so udev applies it to managed links, keeps their creation-time
  address, and renames nothing.
- **TAPs depend on the file, and the bridge carries it as defence in depth.**
  A TAP cannot be created with an address, so the file is what keeps a TAP's
  recorded address unchanged from creation to deletion.
- **The policy is proved where it is installed.** The appliance image test
  and each dev and test substrate's provisioning prove it by behaviour, not by
  the file's presence. A scratch TAP and an addressless scratch bridge, once
  udev reports them initialized, still carry their creation-time addresses,
  and udev names the Overdrive file as applied. A failed check fails the image
  test or the provisioning.

The requirement covers every host link manager. The mechanism here is for
systemd-udevd, the writer the RCA observed. A host where systemd-networkd or
NetworkManager matches a managed name violates the requirement too.

## Alternatives considered

### Owner-assigned TAP addresses

The owner would set each TAP's address and record the value it set. Rejected:
it races udev the same way. A TAP's add event always shows a random address,
so udev acts on it whatever the owner does next, and the owner's write lands
after the add event, inside the window udev races. Bounding the race needs
udev's completion signal, which the runtime does not read.

### A global `MACAddressPolicy=none` for the host

Masking `99-default.link`, or overriding its address policy for every name,
would stop the rewrite without naming managed links. Rejected. It changes the
naming and address policy of every host interface, the physical NICs included,
to fix a problem confined to links Overdrive creates.

The narrower `OriginalName=ovd-*` glob that the RCA probed is rejected for the
same reason, at smaller scale. It also covers `ovd-` links whose address #295
neither records nor audits: the load-balancer veth pair (ADR-0061) and the
test-gated veths. It would change their behaviour for no benefit.

### A repair or reconcile loop

The owner would re-read and re-set until the address is stable, wait a settle
delay, or leave the rewrite to the runtime audit. Rejected, for the reasons
given in *Context*:

- the writer is asynchronous and gives no completion signal, so a loop cannot
  know it has won;
- udev's own latency can exceed any settle delay;
- the deterministic signal needs a udev dependency or a shell-out;
- for TAPs, a repair would erase the tamper detection that the recorded
  address exists for.

### Accept the audit's false VM kills

The host policy would stay unchanged, and the audit would kill any VM whose TAP
udev rewrote after the record. Rejected. On a default host, as the RCA
predicts, this would kill healthy VMs at random. Each kill would look like a
MAC hijack of a workload, so the audit's
damage signal would stop meaning tampering, and a host configuration defect
would surface as workload failures instead of at the host.

### The `.link` file alone, for the bridge too

The owner would keep creating the bridge without an address and rely on the
file. Rejected. A missing or shadowed file would then refuse boot on the
bridge. Creating the bridge with its address makes it immune to udev's address
policy under any host configuration, so the file matters only for TAPs. Both
mechanisms are kept.

## Consequences

Positive:

- A fresh-host boot no longer refuses because udev rewrote the bridge. On any
  host, udev's address policy leaves the bridge and the probe's scratch bridge
  at the fixed address from registration.
- On a compliant host, the TAP address that provision records is the TAP's
  address from creation to deletion. Activation and the audit compare against
  a value nothing changes, so a TAP damage report still means a write by a
  process the platform does not trust.
- No dependency, crate, subprocess, or daemon is added. The runtime never
  reads udev.

Negative:

- **The startup probe gains one condition, the requirement's runtime check.**
  It applies Earned Trust (`brief.md` SD-5): a substrate lie refuses boot
  instead of surfacing as workload failures. The probe records its scratch
  TAP's address and ifindex right after creating it, and re-reads them before
  cleanup. A change refuses startup (`health.startup.refused`). The condition,
  and its rejected alternatives (no boot check; a deterministic runtime proof),
  are in the feature delta.
  - The check is one-sided. It proves only that the scratch TAP's address and
    ifindex were unchanged between the record and the re-read. In the RCA's
    run the scratch TAP lived about 70 ms, against udev's 3.6 ms at info level
    and 28–31 ms at debug level. A writer that acts outside that window, or
    that writes the same value, passes.
  - A refusal is a true positive: the probe changes the scratch TAP only in
    cleanup, after the re-read.
  - The installation check is the deterministic proof. By choice, the runtime
    has no deterministic proof of its own.
- **A host without the file is expected to refuse nearly every boot.** udev
  rewrote the scratch TAP 3.6 ms after creation, inside the TAP's life. On a
  default-policy host the probe condition is therefore expected to catch the
  rewrite on most boots, not only on boots that create the bridge. Each
  substrate must carry the file before the probe condition runs on it.
- **The appliance image carries a requirement.** It must satisfy
  REQ-295-LINKMAC. If it runs systemd-udevd, as the dev and test substrates
  do, it ships the file and runs the installation check in its image test.
  Whether the image runs systemd-udevd is not yet pinned.
  - ADR-0068 makes the image Overdrive's to build, but the image's build layer
    does not exist yet. That layer is the Image Factory MVP,
    [GH #75](https://github.com/overdrive-sh/overdrive/issues/75), which is
    open and has no acceptance criteria written.
  - Until it lands, the requirement is carried by this ADR, the feature delta,
    and the DEVOPS handoff annotation in `brief.md`.
  - ADR-0068 is not amended, because it records a separate decision, the
    kernel pin.
- **The kernel evidence is not yet on the pinned kernel.** The kernel
  behaviour this decision rests on is cited at upstream v7.0 and was observed
  on the Lima kernel `7.0.0-31`. The pinned 6.18 appliance kernel (ADR-0068)
  has not exercised it yet.
- **The dev and test substrates carry the same file and check.** Two hosts
  install it and fail provisioning if the check fails:
  - the Lima dev VM, `infra/lima/overdrive-dev.yaml`, which also provisions
    CI's integration job;
  - the metal host, through `infra/provision/common-system.sh`, which
    `infra/metal/provision.sh` runs.

  The shared Lima VM must be recreated to pick the file up, which interrupts
  runs in other workspaces.
- **A host can violate the requirement and still pass the probe.** On such a
  host:
  - the bridge is still left alone by udev's address policy;
  - a TAP rewritten before provision records its address is recorded
    harmlessly;
  - a TAP rewritten after the record is damage: an activation failure, or an
    audit kill of that VM.
- **The file is coupled to the managed-link names.** Its globs follow the
  bridge and TAP name prefixes. Renaming a managed link, or adding a managed
  link whose address is recorded or audited, means changing the file in the
  image and in both substrates in the same change.
- **Only systemd-udevd is fenced by mechanism.** Another link manager
  configured to match managed names is still a violation. The installation
  check catches it only if it writes before udev reports the device
  initialized. The startup probe catches it only inside the probe's window.
  For the node bridge `ovd-gbr0`, the boot read-back and the runtime audit
  also detect it (ADR-0126).
