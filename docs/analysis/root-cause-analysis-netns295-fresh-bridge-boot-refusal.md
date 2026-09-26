# RCA — `netns-density-295`: fresh-bridge boot refusal and shared-DNS no-reply in S-ND295-00

- **Test:** `crates/overdrive-control-plane/tests/integration/shared_guest_network_startup.rs::production_host_owner_boots_only_after_real_shared_identity_is_exact` (S-ND295-00, active/unmarked).
- **Tree:** HEAD `72c0b7a2`. Historical comparison at `81ea7d47`.
- **Substrate:** Lima `overdrive` VM, aarch64, `uname -r` = `7.0.0-31-generic`, systemd `259 (259.5-0ubuntu3.4)`. The metal host was not used, because Lima reproduced the failure.
- **Run command** (the Phase C C-12b/c shape):
  `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test integration --features integration-tests -E 'test(/production_host_owner_boots_only_after_real_shared_identity_is_exact/)' --no-fail-fast`
- **Kernel source cited** is upstream `torvalds/linux` tag `v7.0`. The running kernel is Ubuntu's `7.0.0-31` build. Every cited behaviour was also observed on that kernel.
- **systemd source cited** is `systemd/systemd` tag `v259`.

## 1. Symptom

The DISTILL Phase C run (`distill/red-classification.md`, blocker 3) recorded two failures of an active body:

1. **C-12b (no `ovd-gbr0` on the host):** boot is refused at `guest_network.rs:4015-4031`:
   `GuestNetworkBoot(PostconditionMismatch { operation: BridgeObserve, expected: BridgeLinkIdentity { …, link_kind: Bridge }, observed: Some(BridgeLinkIdentity { …, link_kind: Bridge }) })`
   Afterwards the bridge carries MAC `b6:f2:51:ad:47:ae`, not `GUEST_BRIDGE_MAC` `02:01:00:00:00:01`. The Debug projection omits the MAC, so expected and observed print identically. Phase C recorded the same refusal on metal (N-04). This RCA did not examine metal, so attributing N-04 to the same cause is an inference.
2. **C-12 and C-12c (bridge present):** boot succeeds, but the shared DNS query to `100.95.0.1:53` gets no reply within 2 s:
   `shared DNS reply: Os { code: 11, kind: WouldBlock }` (`:347`, joined at `:351:6`).

## 2. Scope

- **In scope:**
  - `HostSharedGuestNetworkOwner::converge_shared` (`guest_network.rs:3971-4031`);
  - the `overdrive-netlink` link helpers it calls;
  - host udev link policy;
  - the DNS-responder composition in `run_server`;
  - the test body's DNS oracle.
- **Out of scope, but found and reported in §8:** an intermittent `StartupProbe` timeout. It was not root-caused.

## 3. Evidence: the populations

All runs were on a fresh host (`ovd-gbr0`, `table bridge overdrive-mtls` and the `mtls-endpoints` pins removed), unless marked otherwise.

| Population | Tree | n | BridgeObserve refusal | DNS `WouldBlock` (boot passed) | StartupProbe timeout |
|---|---|---|---|---|---|
| Fresh host, udev at info level (runs 2, 3 and an 8-run loop) | HEAD `72c0b7a2` | 10 | 6 | 3 | 1 |
| Fresh host, udev at debug level (run 1) | HEAD | 1 | 0 | 0 | 1 |
| Bridge already present (runs 4, 5) | HEAD | 2 | 0 | 2 | 0 |
| Fresh host (6 runs) | `81ea7d47` (pre-`b5ef001b`, pre-DISTILL-B) | 6 | 3 | 3 | 0 |

**Correlation.** Every BridgeObserve refusal left the bridge at `b6:f2:51:ad:47:ae`. Every run that got past BridgeObserve left it at `02:01:00:00:00:01`. From the 8-run HEAD loop:

```
fresh run 1: outcome=[operation: BridgeObserve] post-run bridge MAC=b6:f2:51:ad:47:ae
fresh run 2: outcome=[shared DNS reply: Os { code: 11] post-run bridge MAC=02:01:00:00:00:01
fresh run 3: outcome=[operation: BridgeObserve] post-run bridge MAC=b6:f2:51:ad:47:ae
fresh run 4: outcome=[shared DNS reply: Os { code: 11] post-run bridge MAC=02:01:00:00:00:01
fresh run 5: outcome=[operation: BridgeObserve] post-run bridge MAC=b6:f2:51:ad:47:ae
fresh run 6: outcome=[StartupProbe, source: Custom { kind: TimedOut, error: "detached guard packet did not reach the exact drop transition" } }] post-run bridge MAC=cat: /sys/class/net/ovd-gbr0/address: No such file or directory
fresh run 7: outcome=[operation: BridgeObserve] post-run bridge MAC=b6:f2:51:ad:47:ae
fresh run 8: outcome=[operation: BridgeObserve] post-run bridge MAC=b6:f2:51:ad:47:ae
```

The same loop at `81ea7d47`:

```
81ea7d47 fresh run 1: outcome=[shared DNS reply: Os { code: 11] post-run bridge MAC=02:01:00:00:00:01
81ea7d47 fresh run 2: outcome=[shared DNS reply: Os { code: 11] post-run bridge MAC=02:01:00:00:00:01
81ea7d47 fresh run 3: outcome=[operation: BridgeObserve] post-run bridge MAC=b6:f2:51:ad:47:ae
81ea7d47 fresh run 4: outcome=[operation: BridgeObserve] post-run bridge MAC=b6:f2:51:ad:47:ae
81ea7d47 fresh run 5: outcome=[shared DNS reply: Os { code: 11] post-run bridge MAC=02:01:00:00:00:01
```

A sixth `81ea7d47` run, executed before this loop, is counted in the table. It reproduced the exact C-12b refusal:
`PostconditionMismatch { operation: BridgeObserve, expected: BridgeLinkIdentity { name: "ovd-gbr0", ifindex: Some(2603), link_kind: Bridge }, … }`.

## 4. Hypotheses, predictions and pasted output

### P1: `b6:f2:51:ad:47:ae` is systemd-udevd's persistent MAC for the name `ovd-gbr0`

- **Hypothesis:** udev's `MACAddressPolicy=persistent` gives a new `ovd-gbr0` a deterministic MAC.
- **Prediction:** a plain `ip link add ovd-gbr0 type bridge`, with no userspace MAC set, reads `b6:f2:51:ad:47:ae` with `addr_assign_type` 3 within 1 s, on every cycle; `ID_NET_LINK_FILE` is `99-default.link`.
- **Falsification:** the MAC varies across cycles, and `addr_assign_type` stays 1 (kernel random).

The host policy is `/usr/lib/systemd/network/99-default.link`: `[Match] OriginalName=*`, `[Link] … MACAddressPolicy=persistent`.

```
cycle 1: t+0 addr=32:05:29:b8:48:d7 assign_type=1 | t+1s addr=b6:f2:51:ad:47:ae assign_type=3
E: ID_NET_LINK_FILE=/usr/lib/systemd/network/99-default.link
cycle 2: t+0 addr=b6:f2:51:ad:47:ae assign_type=3 | t+1s addr=b6:f2:51:ad:47:ae assign_type=3
cycle 3: t+0 addr=b6:f2:51:ad:47:ae assign_type=3 | t+1s addr=b6:f2:51:ad:47:ae assign_type=3
```

**Confirmed.** The kernel gives the bridge a random MAC (`32:05:…`, type 1), and udev replaces it with the same `b6:f2:51:ad:47:ae` every time. That is exactly the MAC recorded for the C-12b failure.

### P2: on the production path, udev's write lands after the owner's `set_link_mac`

- **Hypothesis:** on the production boot path, udev's persistent-MAC write reaches `ovd-gbr0` after `converge_shared`'s `set_link_mac`, so it overwrites `GUEST_BRIDGE_MAC` before the read-back.
- **Prediction:** a host `ip -ts monitor link`, running during a fresh-host boot that is refused, shows this sequence for `ovd-gbr0`: kernel-random MAC → `02:01:00:00:00:01` → `b6:f2:51:ad:47:ae`, with the last change after the owner's set. `udevadm monitor` shows the `add` event for `ovd-gbr0` completing in the same window.
- **Falsification:** `02:01:00:00:00:01` never appears (the set never landed), or a different value overwrites it, or the refusal happens with the MAC still at `02:01`.

Run 2 (HEAD, fresh host, udev at info level) failed with `PostconditionMismatch { operation: BridgeObserve, expected: BridgeLinkIdentity { name: "ovd-gbr0", ifindex: Some(2551), … }`. The link monitor showed:

```
[2026-09-26T01:31:34.555124] 2551 ovd-gbr0 <BROADCAST,MULTICAST,UP,LOWER_UP> 7a:58:01:1c:33:ef   # created (kernel random)
[2026-09-26T01:31:34.555863] 2551 ovd-gbr0 <BROADCAST,MULTICAST> 7a:58:01:1c:33:ef             # set_link_down
[2026-09-26T01:31:34.556794] 2551 ovd-gbr0 <BROADCAST,MULTICAST> 02:01:00:00:00:01             # set_link_mac (+1.67 ms)
[2026-09-26T01:31:34.556988] 2551 ovd-gbr0 <BROADCAST,MULTICAST> b6:f2:51:ad:47:ae             # udev persistent MAC (+0.19 ms later)
[2026-09-26T01:31:34.558320] 2551 ovd-gbr0 <NO-CARRIER,BROADCAST,MULTICAST,UP> b6:f2:51:ad:47:ae # set_link_up
```

`udevadm monitor --udev` in the same run:

```
UDEV  [191985.496633] add      /devices/virtual/net/ovd-gbr-probe (net)
UDEV  [191985.498047] add      /devices/virtual/net/ovd-tp-probe (net)
UDEV  [191985.566752] remove   /devices/virtual/net/ovd-tp-probe (net)
UDEV  [191985.639536] remove   /devices/virtual/net/ovd-gbr-probe (net)
UDEV  [191985.677163] add      /devices/virtual/net/ovd-gbr0 (net)
```

The anchoring link-monitor lines from the same run:

```
[2026-09-26T01:31:34.371129] 2549: ovd-gbr-probe: <BROADCAST,MULTICAST,UP,LOWER_UP …   # created
[2026-09-26T01:31:34.524620] Deleted 2549: ovd-gbr-probe: <BROADCAST,MULTICAST> …     # deleted
```

These `ovd-gbr-probe` create and delete events anchor the monotonic clock to wall time with an offset of about 191951.115–.125 s, give or take about 5 ms, because a udev event completes after the kernel event. The `ovd-gbr0` `add` event therefore completes at about 34.552–34.562, the window that contains the `b6:f2` write at 34.556988. After the run:

```
2551: ovd-gbr0: <NO-CARRIER,BROADCAST,MULTICAST,UP> … link/ether b6:f2:51:ad:47:ae … bridge_id 8000.b6:f2:51:ad:47:ae … nf_call_iptables 1
addr_assign_type = 3
```

**Confirmed.** The owner's set landed at +1.67 ms. The udev persistent MAC landed 0.19 ms later, before the read-back.

**Attribution of the `b6:f2` write (value fingerprint).** No udev log was captured for the losing `ovd-gbr0` instance. The writer is identified by the value it wrote:

- `b6:f2:51:ad:47:ae` is the output of systemd's persistent-MAC derivation for this machine and the interface name `ovd-gbr0`. P1 shows udev writing this value, with `ID_NET_LINK_FILE=99-default.link`, on every bare create.
- Every Overdrive writer on this path writes only `GUEST_BRIDGE_MAC` (`guest_network.rs:3979`, `:1172-1174`, `:1962`).

The other candidate writers were excluded as follows:

- **systemd-networkd** is active, but its only catch-all `.network` (`/run/systemd/network/zzzz-dracut-default.network`) matches `Kind=!*`, that is, only links with no kind. It therefore never matches a bridge or tun, and it has no `MACAddress=`.
- **NetworkManager** is `inactive`.
- **Other Conductor workspaces** share the VM and the node-global name `ovd-gbr0`. `pgrep -af "cargo|nextest|rustc"` in the VM was empty before the first run and before the historical build. That check does not cover every moment of the 8-run loop.

A more direct writer-identity probe for a follow-up would be a `bpftrace` kprobe on `dev_set_mac_address_user` that prints `comm`/`pid`. It would perturb udev far less than debug logging.

**Direct udev decision log.** Run 1 had udev at `--log-level=debug`. In that run the startup probe failed first (§8), so `ovd-gbr0` was never created. The log still shows both branches of the decision on the probe links:

```
ovd-gbr-probe: Device has addr_assign_type attribute: 3
ovd-gbr-probe: MAC address on the device already set by userspace.
ovd-tp-probe:  Device has addr_assign_type attribute: 1
ovd-tp-probe:  Config file /usr/lib/systemd/network/99-default.link is applied
ovd-tp-probe:  Using "ovd-tp-probe" as stable identifying information
ovd-tp-probe:  Applying persistent MAC address: 62:a6:68:1e:6f:3f
```

Run 1's journal shows the debug-level processing latency:

- `ovd-gbr-probe: Device is queued` at `.052864`, and `Device has addr_assign_type attribute: 3` read at `.084359`: about 31 ms;
- `ovd-tp-probe` queued at `.053821`, and the attribute read at `.081672`: about 28 ms.

In run 2, without debug logging, udev rewrote the probe TAP 3.6 ms after creation. Debug logging is therefore expected to move udev's read past the owner's set at +1.7 ms and hide the bridge race. That is **unverified for the bridge**, because run 1 never created `ovd-gbr0`. The losing `ovd-gbr0` instance was captured with the link and udev monitors instead.

### P3: the race is a timing window that does not depend on Overdrive code

- **Hypothesis:** udev reads `addr_assign_type` early. A userspace MAC set that lands after the read but before udev's write is overwritten.
- **Prediction:** with plain `ip` commands and no Overdrive code involved, a create-then-set sequence with a set gap of about one `exec` loses sometimes. A single-process `ip -batch` sequence (gap under 0.5 ms) never loses.
- **Falsification:** there are no losses at any gap.

`ip -batch` (create → down → set within about 0.4 ms), 6 trials: all final `02:01:00:00:00:01`. The monitor shows `create … d2:86:df:99:2d:4a` followed 0.43 ms later by `02:01:00:00:00:01`, with no later change.

Separate `exec`s, each trial pre-warmed by adding and deleting a dummy link 150 ms earlier. Trial 1 adopted run 2's leftover bridge, so 9 trials were fresh creates:

```
trial 5 final=b6:f2:51:ad:47:ae
trial 8 final=b6:f2:51:ad:47:ae
(7 other fresh trials final=02:01:00:00:00:01)
```

**Confirmed: 2 of 9 lost.** The loss depends on timing. The production owner creates and then sets over six netlink messages, `ensure_bridge` (GET, NEWLINK) → `set_link_down` (GET, SET) → `set_link_mac` (GET, SET), at about 1.7 ms. That falls inside the window. Loss rates were 6 of 9 HEAD boots that reached BridgeObserve, and 3 of 6 at `81ea7d47`. A repeat of the same shape inside P9's control arm (§9) lost 0 of 10. That is *consistent with* a timing-sensitive window, but it is not statistically distinguishable from 2/9, and it may simply be a control that failed to reproduce.

**Supporting inference, not independently proven (warm-worker mechanism).** In run 2 the startup probe's `ovd-gbr-probe`, the first link created, kept its set MAC. The `ovd-tp-probe` TAP was rewritten 3.6 ms after creation. `ovd-gbr0`, created about 30 ms after the probe bridge was deleted, was rewritten 1.86 ms after creation. This is consistent with udev reusing an idle worker from the probe's events; run 1's log shows `Worker [665117] is forked` for the probe bridge. That would explain why the node bridge loses more often than the probe bridge.

### P4: rejected alternatives for question 1

| Candidate | Evidence | Verdict |
|---|---|---|
| The address is never set | Monitor line `34.556794 … 02:01:00:00:00:01`. `set_link_mac` sends `RTM_SETLINK` with `IFLA_ADDRESS` (`client.rs:474-482`, rtnetlink 0.23 `LinkUnspec::address` → `LinkAttribute::Address`). | **Falsified.** |
| Set, then reset by a later owner operation | The only later owner operations are `converge_addr`, `set_link_up`, and a sysfs `nf_call_iptables` write (`guest_network.rs:3980-3982`). None carries an address. The overwriting value is udev's deterministic persistent MAC (P1), and it arrives 0.19 ms after the set, before `set_link_up`. | **Falsified** for owner operations. The reset comes from an **external** writer, udev. |
| The kernel changes the bridge MAC on first enslave | No port is enslaved before the read-back: the monitor shows no `master ovd-gbr0` event, and TAPs attach only later, per allocation. Even if one were, `br_stp_recalculate_bridge_id` returns early for a user-set address: `if (br->dev->addr_assign_type == NET_ADDR_SET) return false;` (`net/bridge/br_stp_if.c:263-265`, called from `br_add_if` `br_if.c:669` and `br_del_if` `br_if.c:731`). `NETDEV_PRE_CHANGEADDR` from a port is likewise a no-op (`net/bridge/br.c:78-80`). The netlink set marks the device `NET_ADDR_SET` (`net/core/dev.c:10021` in `netif_set_mac_address`, `:10001-10024`). | **Falsified.** The design's "bridge-MAC takeover is already closed" analysis (HEAD `feature-delta.md:2269-2279`) holds for ports. It did not consider a userspace writer that acts before the kernel marks the address as set. |
| The read-back happens before the set | `observe_link_identity` (`guest_network.rs:3989-3998`) runs after the `block_on_host_netlink` closure, which awaits every ACK (`:3974-3987`). The monitor shows the set before the refusal. | **Falsified.** |

## 5. Root cause (5 Whys)

```
PROBLEM: On a host whose systemd-udevd applies the default 99-default.link,
ordinary `run_server` boot refuses at BridgeObserve in about half of the boots that
must create ovd-gbr0, that is, the first boot after the bridge is absent or was
cleaned up (6 of 9 HEAD boots that reached BridgeObserve; 3/6 at 81ea7d47).
A boot that adopts an existing bridge passes this check.

WHY 1A  converge_shared's read-back sees ovd-gbr0's MAC = b6:f2:51:ad:47:ae ≠ GUEST_BRIDGE_MAC.
        [guest_network.rs:4015-4031; run-2 `ip -d link`: link/ether b6:f2:51:ad:47:ae]
 WHY 2A The owner's set_link_mac(02:01…) landed and was overwritten 0.19 ms later by the
        udev persistent MAC for "ovd-gbr0".
        [P2 monitor; P1 derivation]
  WHY 3A udev net_setup_link reads addr_assign_type when it processes the add uevent. It saw
         NET_ADDR_RANDOM, so it computed a persistent MAC and applied it with RTM_SETLINK after
         the owner's set.
         [systemd v259 link-config.c:414 (sysattr read at processing); :578-686
         link_generate_new_hw_addr (NET_ADDR_SET → skip at :605-606; RANDOM → persistent,
         "Applying … MAC" :679); :688-706 link_apply_rtnl_settings;
         80-net-setup-link.rules:9 IMPORT{builtin}="net_setup_link"]
   WHY 4A A bridge is registered with a random MAC and NET_ADDR_RANDOM. The add uevent is
          emitted inside register_netdevice, before the creator's RTM_NEWLINK is ACKed. So udev
          can read NET_ADDR_RANDOM before any follow-up set can mark the address NET_ADDR_SET.
          [br_device.c:484 eth_hw_addr_random → etherdevice.h:275-282 (NET_ADDR_RANDOM at
          :281); dev.c:11327 register_netdevice → :11441 netdev_register_kobject →
          net-sysfs.c:2358 device_add → drivers/base/core.c:3671 kobject_uevent(KOBJ_ADD);
          RTM_NEWLINK notification only at dev.c:11493]
    WHY 5A ROOT CAUSE A: the owner creates the bridge without its identity and then sets it,
           as create → down → set → read-back once (client.rs:450-460 `ensure_bridge` builds
           `LinkBridge::new(name)` with no address; guest_network.rs:3977-3979). The design
           assumed the node's MAC has one writer: "A live MAC is never adopted",
           feature-delta.md § C-295-0, HEAD :7188-7214. It did not account for the host's udev link policy as a
           second writer. Only the owner converges one fixed MAC, but on a host with udev, a
           newly created link has a second writer during its first milliseconds. Neither the
           design (C-295-0) nor host provisioning (infra/lima/overdrive-dev.yaml,
           infra/metal/provision.sh) fences that writer off, and within the boot window
           nothing re-converges after it writes: converge_shared sets once, then reads back
           once and refuses.
           [git -L: create→set sequence unchanged since c60cdd3b (2026-09-20); read-back since
           6fdae90d (2026-09-21); `grep udev|MACAddressPolicy|.link` finds no link-policy
           handling in docs/ crates/ infra/]
```

Two other call sites share the shape. They are not the refusal site, but the fix must cover them:

- **Per-attach reassert.** Outside the boot window the owner already reasserts `GUEST_BRIDGE_MAC` after every membership mutation (`attach_tap_to_bridge`, `guest_network.rs:1953-1963`), and `audit_shared` checks the bridge MAC (`:4205-4226`). A bridge rewritten by udev at boot would therefore be repaired at the first TAP attach, but only if boot had not already refused.
- **Scratch probe.** The probe's `ConvergeBridge` arm (`:1168-1178`) has the same create-then-set race on `ovd-gbr-probe` and no MAC read-back. It is silent today.

**Backwards validation.** If the owner sets the MAC only after the uevent is emitted, a udev worker that reads before the set and writes after it produces exactly the monitor sequence in P2. Whether that worker was warm is the unproven inference in P3. The same mechanism explains why:

- a bridge created with its address at birth did not lose in 10 trials (P8, §9), and is born `NET_ADDR_SET`;
- an already-present bridge never loses, because no `add` uevent fires on adoption (runs 4 and 5; C-12c);
- retrying the boot "fixes" it (the adopt path).

Root cause A explains the refusal on every fresh-host population. It does not contradict branch B below.

### Consequence beyond the bridge: TAP host-side MACs

TAPs cannot be created with an address. `tun_net_init` calls `eth_hw_addr_random` (`drivers/net/tun.c:1332`), and rtnetlink creation is refused: `"tun/tap creation via rtnetlink is not supported."` (`tun.c:2283`). On this host udev therefore always rewrites a new `ovd-tp-*` MAC, asynchronously, a few ms to tens of ms after creation:

```
run 2:  ovd-tp-probe  4a:8f:cd:1e:0f:1a → 62:a6:68:1e:6f:3f   (+3.6 ms after creation, already enslaved)
P9:     TAP: t0 3e:a5:25:ea:ac:a1 type=1 -> t+1s 3e:8f:b5:47:94:39 type=3      (default policy)
```

D-295-R22/R21 has `provision` record "the MAC read back when `provision` observed the TAP down" as `host_mac` (HEAD `feature-delta.md:1619-1631`, `:1729`). `activate` and every audit then require the live MAC to equal it; a mismatch is per-allocation damage and the VM is killed (HEAD `feature-delta.md:1604`, `:1753`). Once 06-02 lands, the same root cause will make that record race with udev. The likely results are spurious activation refusals or R14 VM kills. **This is not yet reachable at HEAD, because no host-MAC record exists. It is a forward prediction and must be proven by 06-02's RED.**

## 6. Branch B — the DNS no-reply

### P5: DNS no-reply is independent of the bridge MAC

- **Hypothesis:** the no-reply does not depend on the bridge's MAC state.
- **Prediction:** every run that gets past BridgeObserve fails the DNS step, whether the bridge was fresh with the correct MAC (P2's winners) or pre-existing.
- **Falsification:** some run with a correct bridge MAC gets a reply.

Runs 3 (fresh, race won; the monitor shows the bridge created `0e:1d:1d:0c:de:b3`, then `02:01:00:00:00:01` with no later change), 4 and 5 (bridge pre-existing), the 2 HEAD loop winners, and the 3 `81ea7d47` winners all failed with `shared DNS reply: Os { code: 11, kind: WouldBlock … }`. That is 8 of 8 runs that reached DNS. **Confirmed: independent of (1).**

### P6: nothing listens on `:53`; the kernel answers the query with ICMP port unreachable

- **Hypothesis:** in this body's composition no DNS responder is bound, so the kernel rejects the query with ICMP port unreachable, which an unconnected UDP client never observes.
- **Prediction:** a capture shows the query followed immediately by `ICMP 100.95.0.1 udp port 53 unreachable`. A poller that watches for any non-`systemd-resolved` `:53` socket for the whole server lifetime sees none.
- **Falsification:** there is no ICMP, or a `:53` socket appears.

Positive control of the poller predicate against a wildcard `SO_REUSEADDR` socket:

```
UNCONN 0 0 0.0.0.0:53 0.0.0.0:* users:(("python3",pid=672917,fd=3))
MATCH
```

Run 5 (bridge present) capture, `tcpdump -i any '(udp port 53 or icmp) and host 100.95.0.1'`:

```
2026-09-26 01:37:39.368216 lo    In  IP 100.95.0.1.44223 > 100.95.0.1.53: 10586+ A? missing. (25)
2026-09-26 01:37:39.368253 lo    In  IP 100.95.0.1 > 100.95.0.1: ICMP 100.95.0.1 udp port 53 unreachable, length 61
```

The poller output file stayed empty for both runs 4 and 5: no responder socket ever existed. **Confirmed.**

### Why no responder is bound

The body's `config()` sets `dataplane_override: Some(SimDataplane)` (`shared_guest_network_startup.rs:157`). Under `integration-tests`, `compose_mtls = config.dataplane_override.is_none() || config.mtls_probe_fault.is_some()` (`lib.rs:6517`), so it is `false` and `state.mtls_worker` is `None`. The whole `DnsResponder` construct/probe/spawn block sits inside `if state.mtls_worker.is_some() {` (`lib.rs:6842` through `:6987`, `DnsResponder::new` at `:6944`), so it never runs. The shared bridge still converges, because `sweep_stale` and `converge_shared` (`lib.rs:6766`, `:6782`) are outside that gate. This explains why the body sees a real bridge but no real DNS. `ServerConfig.guest_dns` is not read by `run_server` at all today; only the field declaration and constructor use it, at `lib.rs:923` and `:1108`/`:1147`.

### The body's oracle is also unsatisfiable against the existing responder contract

Two further problems make the DNS assertion unsatisfiable even once a responder is composed:

- **The name is not a mesh name.** The query name is `missing.`. `wire::decode` rejects any name that is not `<workload>.svc.overdrive.local` with `WireError::NotMeshName` (`dns_responder/wire.rs:115-116`). `answer_datagram` turns that into `None` (`responder.rs:399`), and the serve loop `continue`s without replying (`responder.rs:371`). That is the documented contract: "a malformed or non-mesh query is silently dropped" (`responder.rs:395-397`, since `c6f4ab2a`, 2026-06-26).
- **Step 05-01 does not change this.** 05-01 composes the DNS owner from the required `ServerConfig::new(…, guest_dns)` port. This body injects `SimGuestDnsFactory::default()` (`:161`), whose double "binds no socket and answers no query" (`overdrive-sim/src/adapters/guest_dns.rs:66-67`).

```
WHY 1B  The DNS client times out (WouldBlock) on `A? missing.` to 100.95.0.1:53.   [tcpdump run 5]
 WHY 2B No socket is bound on :53; the kernel returns ICMP port unreachable to an unconnected
        client.                                                          [tcpdump; poller]
  WHY 3B dataplane_override=Some ⇒ compose_mtls=false ⇒ mtls_worker=None ⇒ the DNS block is
         skipped.                                                        [lib.rs:6517, 6842-6987]
   WHY 4B The body's composition (SimDataplane; after 05-01, SimGuestDnsFactory) cannot bind a
          real responder, and its name `missing.` is outside the responder's answer set.
          [test :157, :161, :340; wire.rs:115-116; responder.rs:371, 399]
    WHY 5B ROOT CAUSE B (test-body defect): the body mixes a real-kernel bridge oracle with a
           real-UDP DNS oracle while composing the DNS owner away, and it expects a reply to a
           query class that the responder contract drops silently. It was authored so
           (`dd1a18fe`, 2026-09-20; `git log -L` shows the body unchanged since), and no run has
           ever satisfied it.
```

**Verdict: the DNS no-reply is a separate defect, not a consequence of (1).** It is a DISTILL test-body defect in S-ND295-00, not a production defect. It masks nothing about the bridge. A correct bridge MAC only moves the failure from `:311` to `:351`.

## 7. Regression analysis

`git log -L` over the relevant ranges:

| Code | Introduced | Changed since |
|---|---|---|
| `converge_shared` create → down → set MAC → addr → up (`guest_network.rs:3977-3981`) | `c60cdd3b` 2026-09-20 | No change inside the create→set window. `da7b0047` appended `enable_bridge_nf_call_iptables` after `up`. `b5ef001b` added a `self.allocation_lifecycle.lock().await` acquisition, and `e496722c` (DISTILL WP-2) repositioned it to the head of `converge_shared`. Both positions are **before** the netlink block, outside the window. |
| MAC read-back check (`:4016`) | `6fdae90d` 2026-09-21 (step 02-01) | `90aea574` added `!bridge_identity.up`. |
| Real scratch probe creating host `ovd-gbr-probe` / `ovd-tp-probe` before `converge_shared` (`ConvergeBridge` arm `:1168`) | `da5574d8` 2026-09-21 (step 02-01); before that the arm was `Ok(())` | `b5ef001b` added `TUNSETOFFLOAD` to `create_persistent_tap` on the probe-TAP path. The TAP's MAC is not touched. |
| `ensure_bridge`, `set_link_down`, `set_link_mac` (`client.rs:450-482`) | `c60cdd3b` | Unchanged. `b5ef001b` touched `client.rs` for `tunsetoffload` only. |
| `block_on_host_netlink` (`runtime.rs`) | `ba1d41ed` 2026-08-25 | Unchanged. |
| Body DNS query and `dataplane_override: Some(SimDataplane)` | `dd1a18fe` 2026-09-20 | The body is unchanged. `e496722c` only added the `SimMtlsIntercept`/`SimGuestDnsFactory` constructor arguments. |

**Empirical check.** The fresh-host loop was run at `81ea7d47`, the parent of `db3af700`, which is itself the parent of `b5ef001b`. `db3af700` was tried first, but its integration binary does not compile: `E0407: method 'activate' is not a member of trait 'GuestNetworkProvisioner'` at `mtls_install_fail_closed.rs:453`, a RED spec. `81ea7d47` reproduced both failures (§3): 3/6 BridgeObserve and 3/6 DNS.

**Verdict: neither failure is a regression from `b5ef001b` or from the DISTILL phase-B scaffolds** (`cdf74190`, `e496722c`, `b6d71c25`, `7ecd69bb`, `72c0b7a2`). Both are latent defects of earlier committed work:

- **Bridge refusal:** latent since step 02-01 (2026-09-21). The race existed from `c60cdd3b` onward. The read-back (`6fdae90d`) made it observable as a boot refusal, and the host-netns scratch probe (`da5574d8`) plausibly raises its frequency by warming udev workers (the unproven inference in P3). The adopt path hides it whenever a bridge survives from a previous boot, so any earlier green run on a host that already had `ovd-gbr0` does not contradict this.
- **DNS no-reply:** present since the body was authored (`dd1a18fe`).

**How the body escaped the old 02-01 gate (undetermined).**

- **The record.** Old step 02-01 activated S-ND295-00 (`deliver/roadmap.json:151-156`, "S-ND295-00 … are this step's activation set"). `deliver/execution-log.json` records 02-01 GREEN and COMMIT as `PASS` many times (2026-09-20 to 2026-09-21), but the log carries only phase-level records and no per-test evidence. `review-02-01.md:91-95` reviewed this exact body's bridge-guard assertion, and it records no run.
- **Why a real run should have failed.** Under Lima the body runs as root, and Branch B fails deterministically at every tree examined. The DNS block had the same `mtls_worker` gate at `dd1a18fe`, `da7b0047` and `db3af700` (`git show <rev>:…/lib.rs`: `compose_mtls` at 2969/3043/3726; `if state.mtls_worker.is_some()` at 3234/3350/4051). A root run of this body at 02-01 would therefore have failed.
- **Candidate escape paths.** Nothing in the record distinguishes these:
  1. The pre-commit `nextest-affected` hook runs without `--features integration-tests` (`lefthook.yml:178`), so it never compiles this binary.
  2. The body starts with `if geteuid() != 0 { eprintln!("SKIP …"); return; }` (`shared_guest_network_startup.rs:302-307`), which is a **vacuous PASS** in any non-root run, for example `cargo xtask lima run --no-sudo`.
  3. The pre-push `nextest --workspace --features integration-tests` gate (`lefthook.yml:246-247`) did not run on the relevant pushes.
- **Prevention.** A body that needs root should fail, or carry a reasoned `#[ignore]` in lanes without root, rather than `return` as a pass. A DES GREEN record for an activated Tier-3 body should also carry the test-level result.

The escape path does not change either root cause. It explains only how B survived a GREEN.

## 8. Out-of-scope finding: intermittent `StartupProbe` timeout (not root-caused)

This was seen in 1 of the 8 runs of the HEAD fresh-host loop (1 of the 10 info-level fresh runs in §3), and in the single run with udev at debug level (run 1):
`GuestNetworkBoot(Io { operation: StartupProbe, source: Custom { kind: TimedOut, error: "detached guard packet did not reach the exact drop transition" } })` (`guest_network.rs:1115-1118`).

It is a third intermittent refusal of the same active body. It was not investigated to root cause, and it is **not shown to share Root Cause A**. udev is not excluded as a contributor: the only debug-udev run hit it (1/1, against 1/10 at info level).
- **Hypothesis, for follow-up:** the frame written to the TAP queue fd (`:1078`) enters the bridge before the new port reaches a forwarding state. Carrier-on reaches the bridge through asynchronous linkwatch, so the frame dies before `NF_BR_PRE_ROUTING` and the guard counter never moves. rtnl contention from udev's processing of the probe TAP widens the window. The preceding `for _ in 0..64 { std::thread::yield_now() }` (`:1075-1077`) reads as a timing mitigation for exactly this.
- **Prediction:** `pwru --filter-track-skb` on the marker frame shows a drop in `br_handle_frame` (port state not forwarding) in failing runs, and passage to the nft bridge hook in passing runs.
- **Falsification:** the failing run's skb reaches the bridge prerouting hook.
- **Populations to compare:** udev active with the default policy, against udev with `ovd-*` exempted by a `.link` override (the P9 shape), each over repeated fresh boots.

## 9. Recommended fix shape (description only; no code changed)

**Root cause A: the bridge (production).**

- **Immediate mitigation (dev hosts):** none is needed to restore service; a second boot adopts the bridge and passes. For deterministic Lima and metal evidence, an operator can pre-create the bridge, but that hides the defect, so it is not recommended.
- **Permanent fix, structural:** create the node bridge with its identity at birth. Send `IFLA_ADDRESS = GUEST_BRIDGE_MAC` in the `RTM_NEWLINK` that creates it.
  - **Why this works.** `rtnl_create_link` then sets the address and `addr_assign_type = NET_ADDR_SET` before `register_netdevice` (`net/core/rtnetlink.c:3698-3701`). `br_dev_newlink` registers the device, then applies the same address to `bridge_id` (`br_netlink.c:1569-1573`). The `add` uevent therefore already carries type 3, and udev takes its "already set by userspace" branch (`link-config.c:605-606`). Run 1's log shows that branch firing (`ovd-gbr-probe: Device has addr_assign_type attribute: 3 … already set by userspace.`). The explicit set and read-back stay for the adopt path.
  - **P8 (fix-shape probe).**
    - **Hypothesis:** a bridge created with `IFLA_ADDRESS` is `NET_ADDR_SET` at birth, so udev never rewrites it.
    - **Prediction:** `addr_assign_type` is 3 immediately after `ip link add … address 02:01:00:00:00:01 type bridge`, and the final MAC is `02:01:00:00:00:01` after 0.5 s, on every trial, each pre-warmed 150 ms before by a scratch-link add and delete.
    - **Falsification:** any trial reads type 1 at t0, or ends with `b6:f2:51:ad:47:ae`.

    ```
    trial 1 assign_type@t0=3 final=02:01:00:00:00:01
    trial 2 assign_type@t0=3 final=02:01:00:00:00:01
    trial 3 assign_type@t0=3 final=02:01:00:00:00:01
    trial 4 assign_type@t0=3 final=02:01:00:00:00:01
    trial 5 assign_type@t0=3 final=02:01:00:00:00:01
    trial 6 assign_type@t0=3 final=02:01:00:00:00:01
    trial 7 assign_type@t0=3 final=02:01:00:00:00:01
    trial 8 assign_type@t0=3 final=02:01:00:00:00:01
    trial 9 assign_type@t0=3 final=02:01:00:00:00:01
    trial 10 assign_type@t0=3 final=02:01:00:00:00:01
    ```

    The load-bearing observation is `assign_type@t0=3`: the device is `NET_ADDR_SET` before udev can read it, so udev's own code skips it. The 10/10 final MACs are corroboration only. P8 had no same-session control that lost, so 10/10 alone would not prove the probe was in the racing regime. The P3 control ran in an earlier session and lost 2/9. A crafter's RED for this fix should interleave create-with-address trials with a create-then-set control that loses in the same session.
  - **Interface impact.** This touches the public `overdrive-netlink` `Client::ensure_bridge(name)` signature, a cross-crate interface. Under CLAUDE.md § "Implement to the design", **the shape must be pinned by DESIGN**, not improvised by a crafter. The same pin should cover the scratch probe's `ConvergeBridge` (`guest_network.rs:1168-1178`), which shares the create-then-set shape.
- **Permanent fix, defence in depth, needed for TAPs:** TAPs cannot be born with an address (`tun.c:1332`, `:2283`), so the design must choose one of the following:
  - **(a) Host link policy.** Ship a host link policy that exempts `ovd-*` from udev MAC assignment, for example a `.link` with `[Match] OriginalName=ovd-*` and `[Link] MACAddressPolicy=none`, in Lima/metal provisioning and the appliance image, with a boot probe that verifies it.
    - **P9 (host-policy probe).**
      - **Hypothesis:** a `.link` matching `ovd-*` with `MACAddressPolicy=none` stops udev rewriting new `ovd-*` links.
      - **Prediction:** with the override, a new TAP keeps its kernel MAC and `addr_assign_type` 1 after 1 s, and `ID_NET_LINK_FILE` names the override. Without it, in the same session, the MAC changes and the type becomes 3.
      - **Falsification:** the TAP's MAC changes with the override present.

      ```
      CONTROL (default 99-default.link):
        TAP: t0 3e:a5:25:ea:ac:a1 type=1 -> t+1s 3e:8f:b5:47:94:39 type=3
        bridge create-then-set: 0/10 overwritten
      WITH /run/systemd/network/10-overdrive-rca.link (OriginalName=ovd-*, MACAddressPolicy=none):
        TAP: t0 3a:9a:2f:6e:5f:3c type=1 -> t+1s 3a:9a:2f:6e:5f:3c type=1
        bridge create-then-set: 0/10 overwritten
      E: ID_NET_LINK_FILE=/run/systemd/network/10-overdrive-rca.link
      ```

      The TAP arm is confirmed against a same-session control. The bridge arm is uninformative, because its control also lost 0/10.
  - **(b) Owner-assigned MAC.** Make the TAP host-side MAC a desired value that the owner assigns and converges with a bounded re-read, instead of an observed random value recorded once.

  **Production reach.** No ADR pins whether the appliance image runs systemd-udevd at all (`grep udev docs/product` finds only the `/dev/kvm` and `/dev/net/tun` notes). That question must be grounded as well. The **production composition** reaches the defect on the Lima dev VM, observed here. `converge_shared` is the `run_server` path, and the bare-create race happens before any test code acts, so this is not a test-seam artifact. The metal N-04 refusal has the identical symptom, and it is **attributed to Root Cause A by inference only**: this investigation captured no MAC, `addr_assign_type`, systemd version or `.link` policy on metal.
- **Diagnosability:** the refusal cannot name its own cause. `expected` is built from the observed ifindex (`guest_network.rs:4019-4031`), and `GuestNetworkFact::BridgeLinkIdentity` carries neither `mac` nor `up`. A MAC mismatch, an up mismatch and a kind mismatch therefore all print with expected equal to observed, contrary to `rust.md` § "Distinct failure modes get distinct error variants". The fix should carry the observed and expected MAC and up state in the refusal fact (the existing `GuestNetworkFact::Bridge` already has them), or use distinct mismatch variants, so that a future C-12b names its cause.
- **Early detection:** add a Tier-3 regression body that boots through `run_server` repeatedly on a fresh host.
  - It asserts that the bridge MAC read-back equals `GUEST_BRIDGE_MAC` on every iteration, and that a TAP's host-side MAC is stable across a settle window.
  - A udev pre-warm (create and delete a scratch link about 150 ms before, the P3 shape) is a *heuristic* to raise the hit rate, not an established premise.
  - A single run is not a gate for a timing-window race, so the iteration count must be printed.

**Root cause B: the DNS oracle (test body, DISTILL-owned).** The acceptance designer should re-author the S-ND295-00 DNS leg so that it:

- drives a real responder: after 05-01, inject the host `GuestDnsFactory` rather than `SimGuestDnsFactory`. The DNS leg cannot sit behind a `SimDataplane` override, which today removes the responder entirely;
- queries a name inside the responder's answer set, for example `<absent>.svc.overdrive.local.`, expecting NXDOMAIN with SOA, the transaction ID preserved, and the source pinned to `100.95.0.1`.

Until then the body cannot pass. It is currently active and unmarked, which the DISTILL classification relies on.

**Recommendation to DISTILL: split S-ND295-00.** This body is the only active real-kernel oracle for C-295-0 bridge identity. A pending marker on the whole body would also deactivate the oracle that Root Cause A's fix needs.

- **Bridge-identity leg:** stays active. It is owned by A's step, together with the repeated fresh-host body above.
- **DNS leg:** pended to 05-01 and re-authored with the host DNS factory and a `<absent>.svc.overdrive.local.` NXDOMAIN+SOA oracle.

Test bodies are DISTILL's to author, so the split is DISTILL's decision, not this RCA's.

## 10. Owning DELIVER step

| Defect | Owner |
|---|---|
| **A. Bridge MAC race (production)** | **No proposed step owns it.** The re-roadmap (HEAD `feature-delta.md` § "DELIVER re-roadmap input", 05-01 to 10-05) has no step whose scope includes bridge creation or identity convergence (C-295-0), which was delivered under old step 02-01. S-ND295-00 is "RETAINED — active" (`distill/test-scenarios.md:452`). Every later step's Lima and native RED/GREEN evidence boots `run_server`, and about half of the boots that must create `ovd-gbr0` hit this refusal; that is the first boot after the bridge is absent or cleaned up. Metal N-04 is consistent with this, by inference. The fix must therefore land **before 05-01**, as a new first step or as an explicit precondition added to 05-01. It should own the bridge-identity leg of the split S-ND295-00 body (§9). This is a roadmap and DESIGN decision, which needs the `ensure_bridge` signature pin and the TAP/host-policy choice. It must be surfaced to the user and not assigned unilaterally. The TAP half must be settled no later than **06-02**, where `host_mac` is first recorded, with 06-04's activate comparison and 09-01's audit kill as its consumers. |
| **B. DNS no-reply (test body)** | The **DISTILL acceptance designer** (a test-body/oracle defect). The body's DNS leg first becomes satisfiable at **05-01**, the step that composes the DNS owner through the required `guest_dns` port and deletes `compose_mtls`. Its re-authored body should activate there. |
| **C. `StartupProbe` timeout (§8)** | Unowned, and not root-caused. It needs its own investigation before any step can own it. |

## 11. Environment changes made during this investigation, and cleanup

- **Lima kernel state left by the runs was removed:** `ovd-gbr0`, `table bridge overdrive-mtls`, and `/sys/fs/bpf/overdrive/mtls-endpoints/{maps/*,}`. Scratch links from the probes (`ovd-rca-warm`, `ovd-tp-rca`) were also removed.
- **Left in place:**
  - the empty directory `/sys/fs/bpf/overdrive/probe`, which the startup probe recreates on every run;
  - `table ip nat`, which was already present.
- **Restored to their original state:**
  - udev log level (set to `info` again after the run-1 `debug` capture);
  - `/run/systemd/network/10-overdrive-rca.link` (the temporary P9 override was removed, and udev was reloaded afterwards);
  - background monitors (`ip monitor`, `udevadm monitor`, `tcpdump`, and the poller) were stopped, and `/tmp/rca-*` was removed.
- **Temporary worktree:** `/Users/marcus/rca-netns295-db3af700` was created and removed with `git worktree remove` and `prune`.
- **Shared Lima build cache.** Building the historical tree in the shared `CARGO_TARGET_DIR` (`~/.cargo-target-lima`) collided with this workspace's artifacts. Cargo hashes workspace path packages by their workspace-relative path, and it checks freshness by mtime. As a result, `debug/xtask` and `bpfel-unknown-none/release/overdrive-bpf`, an object without `gh295c_egress`, would have been treated as fresh by every worktree whose sources are older. To restore coherence:
  - `cargo clean -p xtask -p overdrive-{core,dataplane,netlink,store-local,reconcilers,host,worker,control-plane,sim}` ran inside Lima (it removed 85.4 GiB across all workspaces' cached builds of those packages);
  - `~/.cargo-target-lima/bpfel-unknown-none` was deleted;
  - this workspace's BPF object was rebuilt with `cargo xtask bpf-build`, and it contains `gh295c_egress` and `gh295c_endpoint`, 14352 bytes.

  The next build in every Conductor workspace will recompile those crates. Anyone doing a similar historical run should use a separate `CARGO_TARGET_DIR`.
- **No production or test code was changed. Nothing was committed.**
