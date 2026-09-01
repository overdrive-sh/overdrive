# Spike findings — guest-stack transparent-mTLS intercept (tap → nft-TPROXY)

**Probe:** `spike-scratch/increment-n-guest-tap-tproxy/`
**Metal box:** `ubuntu@151.115.99.251`
**`uname -r`:** `7.0.0-29-generic` (x86_64, AMD EPYC 8024P, native `/dev/kvm`)
**cloud-hypervisor:** `v53.0`  ·  **nftables:** `v1.1.6`
**Date:** 2026-08-27

---

## Binary verdict: **WORKS**

A process inside a Cloud-Hypervisor microVM — whose only NIC is a virtio-net
device backed by a **tap inside a per-workload netns**, with **no host `struct
sock`** for its connection — dialed an arbitrary peer `10.99.0.1:9000` that is
**not present on the wire**. The guest's egress was **transparently intercepted
on the host** by the production-shape nft-TPROXY rule matching the host-side veth
ingress (`iifname "hveth0"`), landed on a host `IP_TRANSPARENT` listener that
recovered the **original destination via `getsockname()`**, and a **byte-distinct
REQUEST/RESPONSE round-tripped** back into the guest.

This closes the three residual unknowns the prior-art recon
(`.context/spike-guest-stack-mtls-prior-art.md`) left open — tap-in-netns
provisioning, virtio-net offload survival, and the reply path into the guest —
against a **real guest kernel behind a real virtio-net tap** (a netns cannot
model "no host struct sock"; this used an actual VM).

---

## Predicted-vs-actual (the five empirical hypotheses)

### H1 — Routing: guest SYN traverses tap → netns-forward → veth → hveth0 ingress — **CONFIRMED**

Predicted: `tcpdump -ni hveth0` shows a SYN to `10.99.0.1:9000`. Actual (host
`hveth0` capture, `-tt`):

```
1787863868.596108 IP 10.77.0.2.47736 > 10.99.0.1.9000: Flags [S], seq 41074706, win 64240, options [mss 1460,sackOK,TS val 922531230 ecr 0,nop,wscale 7], length 0
1787863868.596133 IP 10.99.0.1.9000 > 10.77.0.2.47736: Flags [S.], seq 278472937, ack 41074707, win 65160, ...
```

The SYN from the guest (`10.77.0.2`) reaches `hveth0` ingress, and the SYN-ACK
comes back **from the foreign address `10.99.0.1:9000`** — i.e. the host
`IP_TRANSPARENT` listener answering as the original destination. The same frame
was seen one hop upstream on the netns `tapw` capture (`8 packets captured`),
confirming the `tap → wveth0 → hveth0` path end-to-end.

### H2 — rp_filter drops forwarded tap frames — **DID NOT MANIFEST (toggle not required)**

Predicted a possible drop of forwarded tap frames under strict reverse-path.
Actual: the round-trip succeeds with `rp_filter=1` **and** `rp_filter=0` (see
matrix). The load-bearing requirement is the **host return route to the guest
/30** (`10.77.0.0/30 via 10.66.0.2 dev hveth0`), which is needed for the reply
path regardless; once it exists, the strict-mode reverse-path check for src
`10.77.0.2` on `hveth0` resolves and passes. `rp_filter=0` was **not required**.

### H3 — virtio-net TX checksum offload breaks the TPROXY'd handshake — **DID NOT MANIFEST (toggle not required)**

Predicted a stalled handshake from `CHECKSUM_PARTIAL` frames. Actual: the
round-trip succeeds with `ethtool -K tx off` **and** with offload left on (see
matrix). This path does **no header rewrite / NAT**, so there is no
incremental-checksum recompute to be poisoned by a partial checksum — the kernel
validates/recomputes on local delivery via TPROXY sk-assign. `tx off` was **not
required** here. (The `tx off` invariant in `.claude/rules/bpf.md` Rule 2 remains
relevant only to the **XDP incremental-csum NAT path**, which this plaintext
passthrough does not exercise.)

### H4 — getsockname returns the ORIGINAL dst, not the redirect target — **CONFIRMED**

Predicted `getsockname` on the accepted host socket returns `10.99.0.1:9000`, NOT
`127.0.0.1:15000`. Actual (host listener log):

```
[listener 1787863868.596] IP_TRANSPARENT set OK
[listener 1787863868.596] listening on 127.0.0.1:15000
[listener 1787863868.596] ACCEPT peer=10.77.0.2:47736  ORIG-DST(getsockname)=10.99.0.1:9000
[listener 1787863868.596] READ REQUEST (26 bytes): PROBE-REQ-GUEST-TO-PEER-7
[listener 1787863868.596] WROTE RESPONSE (28 bytes): PROBE-RESP-HOST-LISTENER-42
```

TPROXY preserved the original destination; the agent recovers it per-flow.

### H5 — reply path: RESPONSE reaches the guest — **CONFIRMED**

Predicted the host's RESPONSE returns hveth0 → veth → netns → tap → guest.
Actual (guest serial console, PID 1):

```
==== GUEST INIT (pid 1) START ====
  iface: lo (idx 1)
  iface: eth0 (idx 2)
selected NIC: eth0
  eth0 configured 10.77.0.2/30 gw 10.77.0.1
connect() -> 10.99.0.1:9000 ...
CONNECT OK
SENT REQUEST: PROBE-REQ-GUEST-TO-PEER-7
RECEIVED RESPONSE (28 bytes): PROBE-RESP-HOST-LISTENER-42
>>> ROUND-TRIP SUCCESS: guest received host's byte-distinct RESPONSE <<<
GUEST-EXITCODE=0
```

The REQUEST (`...GUEST-TO-PEER-7`) and RESPONSE (`...HOST-LISTENER-42`) are
byte-distinct, so the assertion proves the genuine server→client reply pipe back
into the guest, not an echo. The `hveth0` capture shows the reply data segment
(`P.` seq 1:29 length 28 from `10.99.0.1.9000`) and the guest's final ACK.

---

## Empirical toggle matrix — which toggles were REQUIRED

All four combinations of `{rp_filter in 0,1} x {tx_off in on,off}` **WORK**:

```
RP_FILTER=0 TX_OFF=1  -> WORKS  (getsockname=10.99.0.1:9000, GUEST-EXITCODE=0)
RP_FILTER=0 TX_OFF=0  -> WORKS
RP_FILTER=1 TX_OFF=1  -> WORKS
RP_FILTER=1 TX_OFF=0  -> WORKS
```

**Required toggles: NONE of {rp_filter=0, ethtool tx off}.**

What IS load-bearing (structural, not a "toggle"):

- `modprobe nft_tproxy` — TPROXY is a kernel **module** on 7.0.0-29; the nft
  `tproxy` statement fails to load without it. **REQUIRED.**
- **Host return route** `ip route add 10.77.0.0/30 via 10.66.0.2 dev hveth0` —
  the reply path (and strict rp_filter's reverse-path resolution). **REQUIRED.**
- `net.ipv4.ip_forward=1` **inside the netns** — the routed tap→veth hop.
  **REQUIRED** (routed model).
- `ip rule add fwmark 0x1 lookup 100` + `ip route add local 0.0.0.0/0 dev lo
  table 100` — steer the TPROXY'd (fwmark-stamped) skb to local delivery.
  **REQUIRED.**

---

## The exact working nft ruleset + routing (host netns)

```
table ip overdrive_probe {
	chain prerouting {
		type filter hook prerouting priority mangle; policy accept;
		iifname "hveth0" meta l4proto tcp tproxy to 127.0.0.1:15000 meta mark set 0x00000001
	}
}
```
```
ip rule:   32765:  from all fwmark 0x1 lookup 100
table 100: local default dev lo scope host
host route to guest: 10.77.0.2 via 10.66.0.2 dev hveth0
```

This mirrors production `install_outbound_tproxy`
(`crates/overdrive-worker/src/mtls_intercept.rs`): the same `type filter hook
prerouting priority mangle` chain, the same `iifname <host_veth> meta l4proto tcp
tproxy to 127.0.0.1:<port> meta mark set 0x1` rule, the same
`TPROXY_FWMARK=0x1` / `TPROXY_RT_TABLE=100` / `127.0.0.1` redirect. **The
production egress rule fires literally unchanged over a tap-fed veth.**

---

## Topology diagram (routed /30 — closest to production)

```
[ VM guest ]  eth0 10.77.0.2/30   (ioctl-configured; CONFIG_IP_PNP unset -> no ip= autoconfig)
     | virtio-net  (guest terminates TCP in ITS OWN kernel — NO host struct sock)
[ tap "tapw" ]     10.77.0.1/30  (guest's gateway)   --.
     |                                                  |  netns "probens": ip_forward=1,
[ veth "wveth0" ]  10.66.0.2/30                       --'  default route via 10.66.0.1
     | veth pair
[ veth "hveth0" ]  10.66.0.1/30       HOST netns
     |   ^ nft prerouting: iifname "hveth0" meta l4proto tcp
     |     tproxy to 127.0.0.1:15000 meta mark set 0x1
     |   ^ ip rule fwmark 0x1 -> table 100 (local default dev lo)
     v
[ IP_TRANSPARENT listener 127.0.0.1:15000 ]  -> getsockname() = 10.99.0.1:9000
```

CH boot argv (in the netns):

```
ip netns exec probens cloud-hypervisor --cpus boot=1 --memory size=512M \
  --kernel /var/tmp/spike-increment-n/kernel \
  --cmdline "root=/dev/vda rw console=ttyS0 init=/init panic=1 loglevel=7" \
  --disk path=/run/spike-increment-n/rootfs.ext4 \
  --net tap=tapw,mac=12:34:56:78:9a:bc \
  --serial file=.../console.log --console off
```

---

## Edge cases / notes observed

- **Guest self-configures the NIC.** `CONFIG_IP_PNP` is unset, so there is no
  kernel `ip=` autoconfig. The guest `/init` brings up `eth0` itself via
  `SIOCSIFADDR` / `SIOCSIFNETMASK` / `SIOCSIFFLAGS` + `SIOCADDRT` (default
  route). `virtio_net` and `tun` are built-in (`=y`), so no modules are needed
  guest-side; the console shows `tun: Universal TUN/TAP device driver` and the
  virtio-net PCI device enumerating.
- **CH re-uses a pre-created tap.** `--net tap=tapw` attaches to the persistent
  tap already in the netns; CH warns `Tap tapw already exists. IP configuration
  will not be overwritten.` — expected and benign (the netns owns the tap's IP).
- **Serial console name is arch-specific:** `ttyS0` on x86_64 (a wrong name
  yields *silent* no-console output that reads like a hang — reused from
  increment-a's hard-won note).
- **The `block.rs ReadOnly` / `sector 0` CH warnings are cosmetic** — CH
  auto-detects the raw ext4 image and refuses sector-0 writes; the guest mounts
  `/dev/vda` r/w and runs fine (`EXT4-fs (vda): mounted filesystem ... r/w`).
- **Established-flow re-match:** subsequent segments (ACKs, data, FIN) re-ingress
  `hveth0` and TPROXY matches the **established** socket (not the listener), so
  the full connection lifecycle completes — visible in the 11-packet hveth0
  capture through to FIN/ACK.
- The peer `10.99.0.1` is genuinely absent from the wire (no route to it exists
  anywhere in the topology except the default that lands it on `hveth0`), so a
  successful connect is **proof of interception, not of routing to a real
  server.**

---

## Gate recommendation: **PROMOTE**

The one assumption is proven on a real guest kernel behind a real virtio-net tap,
on the pinned-class metal kernel, against the **literal production rule shape** —
and, better than hoped, **with no empirical toggle required** (rp_filter and
tx-offload are both non-issues for this plaintext-passthrough path). Residual
risk is now implementation wiring, not mechanism.

### What the walking-skeleton promotion wires into production

The probe's host side is **already the production code**: the nft rule the probe
hand-installed is byte-for-byte `install_outbound_tproxy(host_veth,
agent_leg_f_port)` (`crates/overdrive-worker/src/mtls_intercept.rs`) plus the
shared `ensure_shared_routing_infra()` (fwmark rule + `local` route table 100),
all of which already exist and are exercised in tree. The only **new** production
wiring is the **tap-in-netns provisioning for VM-kind workloads** — today
`VmConfig.netns = None` and "Job-kind VMs need no tap"
(`crates/overdrive-core/src/vm/config.rs`), and `[vm]+[service]` is refused at
parse (`workload_spec.rs`). A thin slice: at `start_alloc` for a mesh VM
workload, (1) create the per-workload tap inside the alloc's netns and attach it
to the Cloud-Hypervisor `--net`, (2) extend the existing per-workload `/30` so
the guest gets `workload_addr`/gateway and the netns forwards tap→veth (the veth
+ `ip_forward` + return route the reconciler already provisions for non-VM
allocs), then (3) call the existing `install_outbound_tproxy` on that same
host-side veth. The mTLS/kTLS proxy downstream of the intercepted connection is
already proven (#236) and is not part of this slice — a plaintext round-trip is
the whole skeleton. `modprobe nft_tproxy` must be ensured at agent boot (it
already is on the veth-intercept path).

### Promotion-gate decision

**Gate PENDING** (this dispatch was PROBE-only). Recommendation to the
orchestrator/user: **PROMOTE**. The probe code is retained pending the gate.

---

## Reproduce

```bash
# from workspace root
bash infra/metal/bootstrap.sh ubuntu@151.115.99.251 --sync-only
ssh ubuntu@151.115.99.251 'sudo bash ~/overdrive/spike-scratch/increment-n-guest-tap-tproxy/run.sh'
# toggle matrix: sudo RP_FILTER={0,1} TX_OFF={0,1} bash .../run.sh
```

Probe sources (committed, throwaway until the skeleton lands):
`spike-scratch/increment-n-guest-tap-tproxy/{listener.c,guest-init.c,build.sh,run.sh}`.
