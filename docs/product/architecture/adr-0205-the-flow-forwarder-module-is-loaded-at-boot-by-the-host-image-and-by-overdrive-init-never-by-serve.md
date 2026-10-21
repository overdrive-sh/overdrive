# ADR-0205 — The flow-forwarder module is loaded at boot by the host image and by `overdrive-init`, never by `serve`

## Status

**Accepted — EVMOD-LOAD, approved by the user 2026-10-17.** GH #295. This
ADR decides only which process loads a built module, on the host and in a
Linux guest. How the module is built is ADR-0203 (EVMOD-BUILD); which module
a Linux guest's root filesystem carries is ADR-0206 (EVMOD-GUEST-KIT); the
module's lifetime once loaded — it unloads only when nothing references it —
is ADR-0198 (EVMOD-LIFE with the user's ruling MODULE-REFCOUNT of
2026-10-20, which gives this ADR's development-host reload its idle-module
rule). Each could be
replaced while this placement is kept. A guest that runs no Linux kernel
loads no module; its hop is
[GH #321](https://github.com/overdrive-sh/overdrive/issues/321). Follows
from ADR-0197 (accepted 2026-10-16). The contract, including the guest's
module path and the guest's pre-READY errors, is in the #295 feature delta,
§ *[REF] vsock data path — the in-kernel event-driven forwarder (EVMOD)*,
"Module placement and lifetime".

## Context

- **Only Overdrive loads modules on the appliance** (ADR-0068; A-31), and
  the module set is part of the image.
- **`overdrive serve` does not hold `CAP_SYS_MODULE` today**; nothing else
  it does needs it.
- **In the guest, `overdrive-init` is PID 1 and root before READY**, and
  already loads the vsock transport modules on the stock-kernel test path
  (ADR-0068 § 4, corollary). The guest forwarder must exist before the
  guest admits any flow, and READY is the gate after which flows are
  admitted (§ *Lifecycle Gate Ownership*, G-V7).
- The forwarder's owners open their control context and probe it before
  they admit a flow (ADR-0199): on the host at shared forwarding startup
  (G-V1), in the guest before READY.

## Decision

- **Host.** The appliance image loads the module at boot, before
  `overdrive serve` starts; `serve` never loads a module and gains no
  `CAP_SYS_MODULE`. Nothing on the appliance unloads it. Development
  hosts load it in their test lanes' setup step from the build of the
  workspace that runs the lane. That step loads an absent module, accepts a
  loaded module whose `version` equals the build, and unloads then loads a
  stale module that is idle. It refuses a busy stale module with a "busy,
  retry" message that names the holder class: an open context, with its
  process, or socket references whose free has not run yet. It never asks
  for a reboot. The Lima template and `infra/metal` provisioning do not load
  the module, because several workspaces share one development VM. A
  missing module is seen by `serve`'s forwarder probe, which refuses the
  start (G-V1).
- **Guest.** `overdrive-init` loads the module with `finit_module` from one
  fixed path in the guest root filesystem before READY, beside the vsock
  transport modules it already loads, and then opens and probes its
  context. A guest whose module cannot be loaded exits before READY; no
  flow is admitted on a guest without a forwarder.

## Alternatives considered

- **Load the module from `overdrive serve` at startup.** Gives `serve`
  `CAP_SYS_MODULE`, which it otherwise does not need, and moves a boot-time
  image fact into the control plane. Rejected.
- **A systemd-style module-load unit in the guest.** The guest has no init
  system besides `overdrive-init`, which is PID 1. Rejected.
- **Load the guest module from the host** (a second boot medium the VMM
  attaches, loaded by a guest helper). Still a guest-side load, now from a
  medium the host must assemble per launch; which module the guest gets is
  the question ADR-0206 decides, and its rejected launch-injection option
  covers this. Rejected here.

## Consequences

- The host image carries one more boot step and the development test lanes
  one more setup step. On a development host a module change is a reload
  while the module is idle (ADR-0198); no reboot is needed.
- Workspaces with differing module sources still contend on a shared
  development VM. Each one's setup step reloads its own build whenever the
  module is idle, so a run can find another workspace's build loaded and
  fail its stale check. Rerunning the setup step is the remedy.
- `serve`'s capability set is unchanged.
- A guest that cannot load its module fails the VM's start before READY
  with a console cause, through the existing pre-READY start failure.
