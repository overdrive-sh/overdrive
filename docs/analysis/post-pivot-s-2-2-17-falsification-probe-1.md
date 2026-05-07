# Post-pivot S-2.2-17 falsification probe — probe 1 results

**Date**: 2026-05-07
**Step**: phase-2-xdp-service-map / 09-04
**Status**: Falsification probe FAILED. New probe sequence required per ADR-0045 § 7. Step closed BLOCKED_BY_DEPENDENCY pending follow-up under GH #159.
**Author**: Crafty (nw-software-crafter dispatch)

---

## What this document is

ADR-0045 § 7 defines S-2.2-17 (`real_tcp_connection_completes_through_vip_with_payload_echo`)
as the load-bearing falsification probe for the post-pivot dual-XDP datapath:

> The test should GREEN naturally when the new programs land and the
> attach points move to ingress-on-both-veths. **No test-body change
> is required** — the assertion shape (client `nc` exits 0; payload
> echoed) is post-pivot-correct as written. The `#[ignore]` attribute
> added in the dispatch (per Output 4) lifts when the new programs
> land and the test is unblocked.
>
> If S-2.2-17 still fails after the pivot lands, the diagnosis is a
> *new* bug in the post-pivot programs, not a recurrence of the
> pre-pivot failure mode (the kernel mechanism has been structurally
> removed from the path). A failing post-pivot S-2.2-17 is grounds
> for a fresh probe sequence, not an attempt to patch the pre-pivot
> shape.

Step 09-04 was the falsification probe — lift `#[ignore]`, remove the
`ethtool -K ... off` offload disabling that the pre-pivot architecture
required (per ADR-0045 § 7 acceptance criterion: "production-realistic
veth defaults must work"), run S-2.2-17 in Lima, observe the result.

This document records the probe outcome and the next-probe candidates.

## Probe procedure

1. Removed `#[ignore = "blocked on #159 …"]` from
   `crates/overdrive-dataplane/tests/integration/reverse_nat_e2e.rs::real_tcp_connection_completes_through_vip_with_payload_echo`.
2. Removed the `ethtool -K $iface {tx-checksum-ip-generic,tx,rx,tso,gso,gro} off`
   loop from `crates/overdrive-dataplane/tests/integration/helpers/netns.rs::ThreeIfaceTopology::create`.
3. Ran in Lima:
   ```
   cargo xtask lima run -- cargo nextest run -p overdrive-dataplane \
     --features integration-tests --test integration \
     -E 'test(real_tcp_connection_completes_through_vip_with_payload_echo)'
   ```

Both fixture edits were reverted at step close-out; the `#[ignore]` and
the `ethtool -K` block remain in place on `main` pending probe-2 work.

## Probe result — FAILED

```
test integration::reverse_nat_e2e::real_tcp_connection_completes_through_vip_with_payload_echo ... FAILED

[diag] backend_veth MAC = be:5b:b1:57:82:2f
[diag] pcaps written under: /tmp/ovd-rnat3-1913186

thread '...' panicked at crates/overdrive-dataplane/tests/integration/reverse_nat_e2e.rs:392:5:
client nc exited non-zero (code = Some(1)); stdout = ""; stderr = "";
pcaps = /tmp/ovd-rnat3-1913186
```

Wall-clock: ~7.79 s. Six SYN retransmissions (the kernel's standard TCP
retransmit schedule) before `nc -w 5` timed out.

## Probe-1 evidence — pcap-level

Reading the four per-iface tcpdump captures from the failing run:

### `client.pcap` (client-ns)

ARP exchange resolves `10.0.0.1` (the VIP, also the gateway). Client
sends six SYNs `10.0.0.10:46452 → 10.0.0.1:8080`. **No SYN-ACK ever
returns to the client.**

```
16:02:29.648518 ARP, Request who-has 10.0.0.1 tell 10.0.0.10
16:02:29.648566 ARP, Reply 10.0.0.1 is-at c6:00:95:b1:96:9f
16:02:29.648571 IP 10.0.0.10.46452 > 10.0.0.1.8080: Flags [S], seq 2863601254, … length 0
16:02:30.542487 IP 10.0.0.10.46452 > 10.0.0.1.8080: Flags [S], … (retransmit 1)
16:02:31.571955 IP 10.0.0.10.46452 > 10.0.0.1.8080: Flags [S], … (retransmit 2)
16:02:32.593344 IP 10.0.0.10.46452 > 10.0.0.1.8080: Flags [S], … (retransmit 3)
16:02:33.613522 IP 10.0.0.10.46452 > 10.0.0.1.8080: Flags [S], … (retransmit 4)
```

### `lb_a.pcap` (lb-ns, client-facing veth)

Only ARP and IPv6 multicast/router-solicitation noise. **Zero IPv4 TCP
frames captured here.** This is consistent with `xdp_service_map_lookup`
running at XDP ingress on `lb_veth_a` and consuming the SYN before the
kernel networking stack (which is where tcpdump taps) can see it; the
inbound SYN was successfully redirected by `bpf_redirect` at the XDP
layer.

### `lb_b.pcap` (lb-ns, backend-facing veth)

One IPv6 multicast listener report. **Zero IPv4 TCP frames.** Same
caveat — XDP ingress runs before pcap, so a SYN-ACK that came back
from the backend and was successfully reverse-NAT'd by the new
`xdp_reverse_nat` would NOT appear here. But neither would a non-
reverse-NAT'd SYN-ACK that the backend never emitted.

### `backend.pcap` (backend-ns)

**The six client SYNs ARRIVE at the backend, post-DNAT, with
destination rewritten correctly:**

```
16:02:29.648602 IP 10.0.0.10.46452 > 10.1.0.5.8080: Flags [S], seq 2863601254, … length 0
16:02:30.542636 IP 10.0.0.10.46452 > 10.1.0.5.8080: Flags [S], … (retransmit 1)
16:02:31.572001 IP 10.0.0.10.46452 > 10.1.0.5.8080: Flags [S], … (retransmit 2)
16:02:32.593435 IP 10.0.0.10.46452 > 10.1.0.5.8080: Flags [S], … (retransmit 3)
16:02:33.613582 IP 10.0.0.10.46452 > 10.1.0.5.8080: Flags [S], … (retransmit 4)
```

**Crucially**: backend sends NOTHING back. No SYN-ACK, no RST, no ICMP,
no application-layer reply. The backend's TCP stack received the SYN
and either silently dropped it OR `nc` was not actually accepting it.

## Three-line preliminary diagnosis

The forward path is working — XDP DNAT + `bpf_redirect` is delivering the
SYN to the backend correctly (visible in `backend.pcap`, dst rewritten
to `10.1.0.5:8080`).

The failure is on the response path, with three possible mechanisms:

1. **Backend's TCP stack rejects the SYN at input.** `nc -l 8080`
   binds correctly but the kernel drops the SYN before it reaches the
   socket. Most likely candidates:
   - **TCP/IP checksum invalid**: with production-realistic offload
     settings now in play (TSO/GSO/checksum-offload enabled —
     re-enabled by removing the `ethtool -K … off` block), the
     rewritten frame's checksum may not validate at backend ingress.
     This is the closest analog to the pre-pivot failure mode, just
     in a different position in the path.
   - **rp_filter** in strict mode catching the rewritten src
     `10.0.0.10` (route lookup says traffic to that src should go via
     the backend's default route, not back into the LB).
   - **Conntrack/connection-table edge** — backend has no prior
     connection record for this 5-tuple.

2. **Backend received a malformed/non-validating SYN at L2/L3.**
   Same root mechanism as (1) but earlier in the kernel input pipeline.

3. **`xdp_reverse_nat` at `lb_veth_b` ingress isn't catching the
   response.** Even if the backend somehow sent a SYN-ACK, no traffic
   reaches the client via `lb_a.pcap` → `client.pcap`. But tcpdump's
   capture point is post-XDP-ingress, so a SYN-ACK that was redirected
   away at XDP wouldn't appear on `lb_b.pcap` either. This option is
   only viable if probe 2 confirms the backend DID emit a SYN-ACK.

The asymmetry is the load-bearing signal: **`backend.pcap` shows zero
return packets from the backend's address**. That points at (1) or (2)
much more strongly than (3).

## Probe-2 candidates

Per `.claude/rules/debugging.md` § 10, every probe carries a
hypothesis / prediction / falsification path:

### Probe 2a — `bpftrace` `kfree_skb_reason` inside backend-ns

**Hypothesis**: backend kernel drops the SYN at TCP input (bad
checksum / rp_filter / no_socket).

**Prediction**: a non-zero count of `kfree_skb_reason` events with
reason in `{TCP_CSUM, IP_RPFILTER, IP_INHDR, NO_SOCKET}` during the
SYN burst.

**Falsifies**: count = 0 → the SYN reached the socket and the
backend's `nc` listener is the bug. Investigate the listener
lifecycle / port binding next.

**How**:
```bash
cargo xtask lima run -- bpftrace -e '
tracepoint:skb:kfree_skb /pid != 0/ {
    @[args->reason] = count();
}' &
# (then run the test)
```

Cross-reference reason codes against
`include/net/dropreason-core.h` in the kernel tree.

### Probe 2b — `bpftool prog show id <reverse-nat-id> --json` snapshots

**Hypothesis**: `xdp_reverse_nat` is attached on `lb_veth_b` but
`run_cnt` stays 0 across the test — meaning no return packets reach
the program at all.

**Prediction**: if backend sent a SYN-ACK that hit `lb_veth_b`,
`run_cnt > 0`; if backend never sent one, `run_cnt = 0`.

**Falsifies**: distinguishes "backend silent" from "reverse-NAT
broken." Combined with probe 2a's drop-reason data, narrows the
mechanism.

**How**:
```bash
sysctl -w kernel.bpf_stats_enabled=1
bpftool prog show id <id> --json | jq '.run_cnt, .run_time_ns'
# (run test)
bpftool prog show id <id> --json | jq '.run_cnt, .run_time_ns'
```

### Probe 2c — `pwru --filter-track-skb 'host 10.1.0.5'` per-skb trace

**Hypothesis**: the backend's first response packet (if any) exists
somewhere in the kernel but dies at a hook we are not observing.

**Prediction**: if a SYN-ACK is generated, pwru shows it traversing
backend-ns kernel functions and either reaching `dev_queue_xmit` on
`backend_veth` or hitting a `kfree_skb` site; if not, pwru shows
nothing originating from `10.1.0.5`.

**Falsifies**: distinguishes "SYN-ACK was generated and lost in
transit" from "SYN-ACK was never generated."

**How**:
```bash
cargo xtask lima run -- bash -c '
ip netns exec 3i-bck-a-<pid> pwru --filter-track-skb "host 10.1.0.5" &
# (run test)
'
```

## Decision matrix from probe-2 results

| Probe 2a | Probe 2b | Probe 2c | Likely root cause | Next step |
|---|---|---|---|---|
| reason=`TCP_CSUM`, count > 0 | run_cnt = 0 | no SYN-ACK | Checksum invalid at backend ingress (offload interaction) | Audit XDP DNAT csum incremental update path; consider per-iface offload tuning that's _not_ test-fixture-only |
| reason=`IP_RPFILTER`, count > 0 | run_cnt = 0 | no SYN-ACK | rp_filter strict-mode rejecting rewritten src | rp_filter sysctl tuning is fixture territory; promote it into production guidance |
| reason=`NO_SOCKET`, count > 0 | run_cnt = 0 | no SYN-ACK | `nc` not actually listening | Test fixture issue (`nc -l` race, port already bound, etc.); not a dataplane bug |
| count = 0 | run_cnt = 0 | no SYN-ACK | SYN reached socket but `nc` didn't ACK | Application-level fixture issue |
| count = 0 | run_cnt > 0 | SYN-ACK exists, dies in lb-ns | Reverse path broke after backend egress | Audit `xdp_reverse_nat` body for the actual return-path bug |

## Status

- **Step 09-04 closes BLOCKED_BY_DEPENDENCY pending probe-2.** No
  production code change. Falsification probe attempted; failed; the
  failure is a NEW post-pivot bug per ADR-0045 § 7 (a structural
  recurrence of the pre-pivot kernel mechanism is impossible — the
  IP-forwarder is no longer in the path). The bug is not in the
  TCX-egress dispatcher (which no longer exists). The bug is somewhere
  in the post-pivot dual-XDP datapath OR in the test-fixture's
  network setup.

- **`#[ignore]` and the `ethtool -K … off` block stay on `main`.**
  Working-tree edits to lift them were reverted at step close-out.
  The next dispatch (probe-2 follow-up under GH #159) re-applies them
  in its RED phase before running the next round of probes.

- **Pcap evidence retained**: `/tmp/ovd-rnat3-1913186/{client,lb_a,
  lb_b,backend}.pcap` inside the Lima VM (transient — overwritten on
  next test run; capture before re-running the test if needed for
  archival).

## References

- ADR-0045 § 5 (FIB miss → XDP_PASS), § 7 (S-2.2-17 falsification gate)
- `docs/analysis/e1-bpftrace-results.md` — probes 1–7 from the
  pre-pivot diagnostic chain (analogous methodology, different bug)
- `docs/analysis/root-cause-analysis-s-2-2-17-length-n-tcp-drop.md`
  — Rex's earlier RCA, partially superseded by the empirical chain
- GH #159 — *[2.x] Replace IP-forward + TCX-egress with
  bpf_redirect_neigh datapath* (the tracking issue; probe-2 work
  attaches here)
- `.claude/rules/debugging.md` § 10 — probe / hypothesis / prediction
  / falsification triple
