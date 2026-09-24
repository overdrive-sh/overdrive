# Spike Findings — `netns-density-295` cross-tenant host→guest MAC/FDB steal and the ADR-0142 egress control (increment-z)

## Verdicts

- **STEAL REPRODUCED: YES.** An unprivileged process (uid 4200, `CapEff=0`, no
  `CAP_NET_ADMIN`) holding **only** its own TAP's queue fd redirected another
  guest's host-to-guest plaintext to itself, on a real kernel, by setting its
  TAP's host-side MAC to the victim guest's MAC with `SIOCSIFHWADDR`. The
  attacker process then **read the victim's frames out of its own queue fd**.
- **CONTROL BLOCKS: YES.** An ADR-0142-shaped destination-MAC egress gate
  (nft bridge `output`/`postrouting` keyed on `oifname` + `ether daddr`,
  delivering only the TAP's registered guest MAC, broadcast/multicast exempt)
  dropped both the directed steal and the unknown-unicast flood before they
  reached the attacker, while broadcast delivery was preserved. Disabling
  bridge unicast flooding on the port (`bridge link set … flood off`) closed
  the flood variant independently.

The hazard R5-H1 reasoned from source is real on this kernel, and a dest-MAC
egress gate closes it. This is throwaway defensive evidence on the developer's
own bare-metal box; it proposes no production API and makes no promotion
decision.

## Question tested

On one Linux bridge with two host TAP ports modelling two guest microVM TAPs,
can an unprivileged process (uid 4200, no `CAP_NET_ADMIN`) that holds only a
queue fd of its own TAP redirect the host's traffic destined for a second guest
to its own TAP, by setting its TAP's host-side MAC to the second guest's MAC via
`SIOCSIFHWADDR` — and does a destination-MAC egress filter on the TAP (plus
`flood off`) stop it, including the unknown-unicast flood variant?

## Execution substrate

- Host: `Linux 7.0.0-29-generic x86_64 GNU/Linux`; `systemd_detect_virt=none`;
  native, non-virtualized bare metal (`ubuntu@151.115.99.251`).
- `uname -r` = `7.0.0-29-generic`.
- Python `3.14.4`. Tools: `ip`, `bridge`, `nft`, `/dev/net/tun`.
- No guest VM: host TAPs on a bridge model the microVM TAPs; a frame written to
  a TAP queue fd is injected as if from that guest (the bridge learns the src
  MAC on that port), and an `AF_PACKET SOCK_RAW` frame sent on the **bridge**
  device is the host-transmit path (`net/bridge/br_device.c` `br_dev_xmit`).
- Every run went through `cargo xtask metal run --` (rsync + canonical lease +
  root + fail-closed native x86_64/KVM preflight). No compile-only gate.
- MACs (production shapes): bridge `02:01:00:00:00:01`
  (`GUEST_BRIDGE_MAC`); guest MAC = `0x02:0x00` ++ IPv4 octets
  (`guest_network.rs`) → guest A `100.95.0.2`=`02:00:64:5f:00:02`,
  victim B `100.95.0.3`=`02:00:64:5f:00:03`, unknown C `100.95.0.4`=
  `02:00:64:5f:00:04`. `uid 4200` = `overdrive_core::vm::config::OVERDRIVE_VMM_UID`.

## Precondition confirmed: attacker uid 4200 with NO `CAP_NET_ADMIN`

The attacker was a forked child that attached spiketapa's queue fd (as root,
modelling the launcher), then `prctl(NO_NEW_PRIVS)` + `setgroups([])` +
`setgid(4200)` + `setuid(4200)`. At the moment of the `SIOCSIFHWADDR` ioctl:

```
child_steal={"caps_at_ioctl": {"CapBnd":"000001ffffffffff","CapEff":"0000000000000000",
  "CapInh":"0000000000000000","CapPrm":"0000000000000000","Gid":"4200 4200 4200 4200",
  "Groups":"","NoNewPrivs":"1","Uid":"4200 4200 4200 4200"}, "ioctl_errno":0,
  "ioctl_rc":0, "mac_readback":"02:00:64:5f:00:03", "ok":true}
```

`CapEff=0000000000000000` (no `CAP_NET_ADMIN`, no other cap), `Uid=4200`,
`ioctl_rc=0`, `ioctl_errno=0`. The ioctl succeeded with zero effective
capabilities. **The crux held.**

## Per-step evidence (hypothesis / prediction / actual)

### STEP 1 — setup (bridge + two persistent owner-4200 TAPs)

- **Hypothesis:** a bridge with an explicit MAC plus two persistent owner-4200
  TAPs, enslaved and up, model two guest TAPs; ports forward once a queue
  attaches (carrier on).
- **Prediction:** bridge MAC `02:01:00:00:00:01`; TAPs `tun_flags=0x1802`
  (TAP|NO_PI|PERSIST), `owner=4200`, `master=bridge`.
- **Actual:**
  ```
  bridge_mac_at_create=52:73:c6:c3:fb:92 addr_assign_type=3
  bridge_mac_after_explicit_set=02:01:00:00:00:01 addr_assign_type=3
  bridge_mac_after_enslave=02:01:00:00:00:01 addr_assign_type=3
  bridge_mac_final=02:01:00:00:00:01 addr_assign_type=3
  spiketapa…: address=aa:9a:28:dd:2b:ca tun_flags=0x1802 owner=4200 master=spikebr… flags=[NO-CARRIER,BROADCAST,MULTICAST,UP]
  spiketapb…: address=9a:dd:6c:db:71:ad tun_flags=0x1802 owner=4200 master=spikebr… flags=[NO-CARRIER,BROADCAST,MULTICAST,UP]
  ```
  The explicit bridge MAC is honored with `addr_assign_type=3` (`NET_ADDR_SET`).
  After both queues attach (child on A, parent on B), `bridge -details link
  show` reports both ports `state forwarding`, `learning on flood on`.
  (An initial run hard-failed a strict "MAC must read back" assertion because a
  bare `ip link add type bridge` seeds a *random* MAC; the assertion was relaxed
  to record-and-continue since the bridge MAC is not load-bearing for the
  steal/control verdict — delivery keys on the destination MAC. The diagnostic
  run above confirms the explicit set sticks.)

### STEP 2 — learn each guest MAC (non-local dynamic FDB)

- **Hypothesis:** injecting a guest-sourced frame from each TAP fd makes the
  bridge learn each guest MAC as a dynamic (non-local) FDB entry on its port.
- **Prediction:** `GUEST_A_MAC` learned on spiketapa, `GUEST_B_MAC` on
  spiketapb; both non-permanent.
- **Actual:**
  ```
  fdb_guestA=['02:00:64:5f:00:02 dev spiketapa… master spikebr…']
  fdb_guestB=['02:00:64:5f:00:03 dev spiketapb… master spikebr…']
  ```
  Both entries are learned/dynamic (no `permanent`/`static` flag), on their own
  ports. This is the victim's `br_fdb_update`-learned entry the attack replaces.

### STEP 3 — baseline delivery (the litmus)

- **Hypothesis:** host→guestB is delivered to spiketapb (victim) and NOT to
  spiketapa (attacker).
- **Prediction:** B receives host→guestB; A receives none; host→guestA reaches
  A only.
- **Actual:** host→guestB — `B_victim_hits=3, B_cap_hits=3, A_child_hits=0,
  A_cap_hits=0`. host→guestA — `A_child_hits=3, B_victim_hits=0`. The steal
  would be observable: baseline is cleanly isolated.

### STEP 4 — the attack (`SIOCSIFHWADDR` on the held fd)

- **Hypothesis:** the uid-4200 no-cap holder sets spiketapa's host-side MAC to
  `GUEST_B_MAC`; the bridge moves `GUEST_B_MAC` to spiketapa as `LOCAL|STATIC`.
- **Prediction:** `ioctl rc=0` (no `EPERM`) with `CapEff=0`; FDB `GUEST_B_MAC`
  now on spiketapa `permanent`; bridge device MAC unchanged.
- **Actual:** ioctl `rc=0 errno=0`, `mac_readback=02:00:64:5f:00:03`. FDB moved:
  ```
  fdb_guestB_before_attack=['02:00:64:5f:00:03 dev spiketapb… master spikebr…']            (learned)
  fdb_guestB_after_attack =['02:00:64:5f:00:03 dev spiketapa… vlan 1 master spikebr… permanent',
                            '02:00:64:5f:00:03 dev spiketapa… master spikebr… permanent']   (LOCAL|STATIC on the attacker port)
  tap_a_mac_after=02:00:64:5f:00:03   tap_b_mac_after=9a:dd:6c:db:71:ad
  bridge_mac before=02:01:00:00:00:01 after=02:01:00:00:00:01
  ```
  Confirms research addendum 2 B1.2 verbatim: `br_fdb_changeaddr` →
  `fdb_add_local` deleted the victim's learned entry and installed a `permanent`
  (LOCAL|STATIC) entry for `GUEST_B_MAC` on the attacker's port.

### STEP 5 — post-attack delivery = the steal

- **Hypothesis:** host→guestB is now forwarded to spiketapa; the attacker reads
  the victim's host-to-guest plaintext from its own fd.
- **Prediction:** `A_child_fd` hits>0 for host→guestB; `B_victim` hits==0.
- **Actual:** `A_child_hits=3` (the attacker read three `SPKZ|stealB|*` frames
  destined to `02:00:64:5f:00:03` out of its own queue fd), `B_victim_hits=0`.
  **STEAL REPRODUCED.**

### STEP 6 — flood-leak variant (unknown unicast, no MAC change)

- **Hypothesis:** host→guestC (a MAC never learned) is flooded to every
  flood-enabled port, so the attacker receives it with no MAC change at all.
- **Prediction:** both A and B receive host→guestC.
- **Actual:** `A_child_hits=3` and `B_victim_hits=3`. **FLOOD LEAK REPRODUCED**
  (research addendum 2 B1.5 observation 1 / `br_forward.c` flood path).

### STEP 7 — the control (nft egress dest-MAC gate)

- **Hypothesis:** an ADR-0142-shaped egress dest-MAC gate delivering only the
  registered guest MAC (mcast/bcast exempt) drops the stolen and flooded frames
  before they reach the attacker; production form is TCX egress.
- **Prediction:** with the gate active, `A_child==0` and `A_cap==0` for
  host→guestB (stolen) AND host→guestC (flood); a drop counter increments;
  broadcast still passes.
- **Actual:** rules installed on both `output` and `postrouting` hooks per TAP
  (`oifname … ether daddr & 01:00:00:00:00:00 == 00:00:00:00:00:00 ether daddr
  != <registered guest MAC> counter drop`):
  - ctrlStealB (host→guestB, FDB still poisoned to spiketapa): `A_child_hits=0,
    A_cap_hits=0`.
  - ctrlFloodC (host→guestC): `A_child_hits=0, A_cap_hits=0`.
  - ctrlBcast (broadcast): `A_child_hits=3` — broadcast still delivered.
  - Counters — **the `output` (NF_BR_LOCAL_OUT) chain caught everything**:
    ```
    output  oifname "spiketapa…" … != 02:00:64:5f:00:02 counter packets 6 bytes 270 drop
    output  oifname "spiketapb…" … != 02:00:64:5f:00:03 counter packets 3 bytes 135 drop
    post    oifname "spiketapa…" … != 02:00:64:5f:00:02 counter packets 0 bytes 0   drop
    post    oifname "spiketapb…" … != 02:00:64:5f:00:03 counter packets 0 bytes 0   drop
    ```
    tapA's 6 drops = 3 ctrlStealB + 3 ctrlFloodC; tapB's 3 = the ctrlFloodC
    copies flooded to it. The `postrouting` chain saw nothing because the frame
    was already dropped at `output`. **This confirms ADR-0142's own claim that
    the nft-equivalent "observes the same frame at `NF_BR_LOCAL_OUT`."**

### STEP 8 — independent flood control (`bridge … flood off`)

- **Hypothesis:** independently of nft, disabling bridge unicast flooding on the
  attacker port stops the unknown-unicast leak while leaving broadcast intact.
- **Prediction:** with nft removed and `flood off`, host→guestC no longer reaches
  A; broadcast still reaches A.
- **Actual:** regression check with nft removed — flood leaks to A again
  (`A_child_hits=3`). After `bridge link set spiketapa flood off` (`bridge
  -details` shows `flood off`): host→guestC `A_child_hits=0, A_cap_hits=0`;
  broadcast `A_child_hits=3`. **Flood variant closed at the bridge level;
  broadcast preserved.**

### STEP 9 — teardown

`final_residue={bridge_exists:false, nft_exists:false, tap_a_exists:false,
tap_b_exists:false, tap_a_holders:[], tap_b_holders:[]}`;
`cleanup_complete=True`. No `spikebr*`/`spiketap*`/`spikeguard*` residue left.
The pre-run sweep found no leftovers (`SWEEP removed=[]`).

## Edge cases

- **Flood-leak variant (no MAC change required).** Confirmed: unknown-unicast
  to a never-learned guest MAC is flooded to every flood-enabled port, so a
  compromised VMM leaks another guest's host-to-guest frames during the windows
  when the victim's entry is absent (after ageing, before its first frame, or
  after a MAC revert). The nft dest-MAC gate blocks it, and `flood off` blocks
  it independently.
- **Bridge-MAC takeover is prevented by the explicit bridge MAC.** With the
  bridge MAC set explicitly (`addr_assign_type=3` / `NET_ADDR_SET`), the
  attacker changing its port MAC to the numerically lower `GUEST_B_MAC` did
  **not** move the bridge's own MAC (`before=after=02:01:00:00:00:01`). This
  positively demonstrates the ADR-0126 fixed-bridge-MAC protection against the
  gateway-MAC-move DoS (research addendum 2 B1.5 observation 2).
- **Stickiness.** The stolen entry is `permanent` (LOCAL|STATIC); the victim
  cannot re-learn over it. The egress gate stops the *leak to the attacker* but
  does not by itself revert the poisoned FDB entry — consistent with ADR-0142's
  Negative note that the complementary host-MAC audit/kill (ADR-0130 / R14) is
  what clears the poisoning and restores the victim.
- **Which hook.** The egress observation/drop point is `NF_BR_LOCAL_OUT` (the
  nft bridge `output` hook), not `postrouting`. The production form is TCX
  egress on `__dev_queue_xmit`, which sits on the same host→guest leg.

## Design implications for ADR-0142

1. **The control is justified.** Without an egress dest-MAC gate, a compromised
   VMM (uid 4200, no `CAP_NET_ADMIN`, own-queue fd only) both *directs* another
   guest's host-to-guest plaintext to itself (sticky FDB steal) and *passively
   receives* it via unknown-unicast flood. Both are reproduced on a real kernel.
2. **A registered-MAC egress gate closes both.** Delivering host-originated
   unicast only when the destination MAC equals the TAP's *registered* guest MAC
   — decided by the registered record, not the learned FDB — stops the steal
   even while the FDB is poisoned, and stops the flood. This probe proved the
   mechanism with the nft form the ADR names as its functional equivalent; the
   ADR's chosen production form (a per-TAP TCX egress classifier keyed on the
   ADR-0115 endpoint map) sits on the same `NF_BR_LOCAL_OUT`/`__dev_queue_xmit`
   host→guest leg and is expected to behave identically. A native RED against
   the TCX form remains the DESIGN's own required evidence (ADR-0142
   Consequences).
3. **`flood off` on managed ports is a real, independent belt-and-suspenders.**
   It closes the unknown-unicast flood at the bridge without affecting
   broadcast/multicast — matching the ADR's "additionally has bridge unicast
   flooding disabled."
4. **Broadcast/multicast must stay exempt.** The mask `ether daddr &
   01:00:00:00:00:00 == 00:00:00:00:00:00` (unicast only) preserves ARP/ND; the
   probe confirmed broadcast still reaches the port under both controls.
5. **The complementary audit is still needed.** The egress gate does not revert
   the poisoned `permanent` entry; the ADR-0130 host-MAC audit + R14 kill scope
   remain necessary to restore the victim (this probe does not exercise them).
6. **Fixed bridge MAC (ADR-0126) is confirmed load-bearing** — it prevents the
   attacker's port-MAC change from moving the guests' gateway MAC.

## Probe

- Path: `spike-scratch/increment-z-netns-density-295-mac-fdb-20260924T021346Z/spike.py`
- SHA-256: `edf73cca204dd537082d065d030a503dd26fd00b62b555bdac103d34216cfbd4`
- Evidence (complete, append-only, timestamped):
  - `evidence/00-sweep-20260924T021915Z.log` — pre-run sweep + connectivity.
  - `evidence/01-run-20260924T021932Z.log` — `--no-sync` refusal (stale digest).
  - `evidence/01-run-20260924T021950Z.log` — first run; STEP1 strict-assert
    failure (relaxed thereafter); clean cleanup.
  - `evidence/02-run-20260924T022231Z.log` — authoritative full run; both
    verdicts YES; `cleanup_complete=True`.

This probe is throwaway (`.claude/rules/spike.md`): no file under `crates/` was
created or modified, nothing builds or gates on it, and it is deleted when the
implementation it validates lands. No promotion decision is recorded here — the
orchestrator runs the gate with the user.
