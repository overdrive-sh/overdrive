# V-16 — Hybrid unframe with UDP segmentation offload enabled (A-20, D5a, ADR-0165)

**Verdict:** D5a's hybrid unframe **HOLDS** on a USO-enabled interface for every datagram sent without `UDP_SEGMENT`. It is **FALSIFIED** for a kept-frame datagram sent through a socket with `UDP_SEGMENT` set — the program drops it. A new constraint on the owner's host sockets, not a break of the hybrid mechanism.

Probe and raw captures: `spike-scratch/netns-density-295-v16/` (`ebpf/src/main.rs`, `probe/src/main.rs`, `build-on-metal.sh`, `run-veth.sh`, `run-nic.sh`, `verify_pcap.py`, `control-csum.sh`, `capture-csum.sh`, `out/`).

## Substrate

```
host   : Linux 7.0.0-29-generic x86_64, systemd-detect-virt: none
uplink : eno12399np0 bnxt_en, fw 229.2.75.0, MTU 1500, tx-udp-segmentation: off [fixed], generic-segmentation-offload: on
veth   : v16-a <-> v16-b (netns v16-rx), MTU 1500, tx-udp-segmentation: on, tx-gso-list: on
Lima   : 7.0.0-34-generic aarch64 virtio_net, tx-udp-segmentation: off [fixed] (not run)
```
Program: `eg_decode_frag` from the V-11 spike (aya-rs, TCX egress, tuple mode 3 hybrid/lenient), logic unchanged; only observation counters added. Legacy clsact also run for parity.

**Fidelity.** The program runs in `sch_handle_egress` before `validate_xmit_skb` and the driver, so UDP segmentation (hardware or software) happens after it. `NETIF_F_GSO_UDP_L4` only decides whether the skb the program sees is a GSO super-packet, and the socket (`UDP_SEGMENT`) decides that. veth exercises the TC-visible side; bnxt was used for wire evidence (program attached ~30 s per run, gated to TEST-NET-2 tuples, detached and verified).

## Verdicts

| Sub-claim | Verdict | Bound |
|---|---|---|
| V-16a: every size class through `tx-udp-segmentation: on` arrives byte-exact (empty, 1 B–65,507 B, every MTU boundary, fragmented, frame-shaped, near-miss) | HOLDS | veth; 7.0.0-29 x86_64 |
| V-16b: same through the physical NIC (bnxt_en), checked on the wire | HOLDS | NIC USO `off [fixed]`; wire capture only |
| V-16c: toggling `tx-udp-segmentation` changes nothing for plain `sendto` | HOLDS | identical counters; no GSO skb reaches the program |
| V-16d: kept-frame datagram through a socket with `UDP_SEGMENT` | **FALSIFIED** | only sockets that set `UDP_SEGMENT` |
| V-16e: a real hardware-USO NIC | INCONCLUSIVE | none available |

## Key evidence

veth matrix (`out/veth-matrix-metal-x86_64.log`):
```
uso-on         SUMMARY byte-exact=69 fail=5 total=74   (the 5 FAILs are all UDP_SEGMENT + kept frame)
uso-off        SUMMARY byte-exact=69 fail=5 total=74
uso-on-legacy  SUMMARY byte-exact=69 fail=5 total=74
uso-on-unreg   SUMMARY byte-exact=45 fail=29 total=74  (control: tuples not registered)
RESULT BYTE-EXACT id=0  Empty   n=0     wire=8     got=3/3
RESULT BYTE-EXACT id=38 Escaped n=1465  wire=1473  got=3/3
RESULT BYTE-EXACT id=50 Escaped n=65499 wire=65507 got=3/3
tc_decoded=66 tc_decoded_empty=4 frag_first_unframed=39 frag_shifted=393 lenient_pass=115 gso_skbs_seen_first_frag=12
```
All 56 non-`UDP_SEGMENT` cases byte-exact in every registered variant. Datagrams above MTU are still IP-fragmented before TC on a USO device; the fragment-aware path delivered them.

NIC wire (`out/nic-reg-metal-x86_64.log`):
```
WIRE-SUMMARY byte-exact=69 fail=5 total=74  ip_packets_on_wire=1288 fragment_packets=1122 max_frame=1514 unfinished_reassemblies=0
```
Unregistered NIC control: `byte-exact=45 fail=29`, `tc_decoded = 0`.

UDP checksum: with software TX checksum and a validating receiver, 56/56 byte-exact; edited datagrams carry checksum 0 (valid IPv4 "no checksum"), untouched ones a valid sum; control proves the receiver drops a bad checksum (`UdpInCsumErrors 1`).

V-16d:
```
RESULT FAIL id=68 Escaped n=1200  wire=1208  gso=true datagrams=0
RESULT FAIL id=72 Escaped n=59000 wire=59008 gso=true datagrams=0
COUNTER tc_err_shot = 5   fail_adjust_room = 5   sum_of_negated_errnos = 2620 (= 5 × ENOTSUPP 524)
```
`bpf_skb_adjust_room(-8)` is refused on a UDP GSO skb (shrinking `gso_size` unsupported for UDP); the error arm returns `TC_ACT_SHOT`. Bare payloads with `UDP_SEGMENT` pass untouched. Experiment with `BPF_F_ADJ_ROOM_FIXED_GSO`: program unframes (`gso_skbs_unframed = 5`) but nothing is delivered; loss moved after the program, cause not isolated — not a cheap fix.

Reachability: nothing in D5a / ADR-0165 says owner host sockets use `UDP_SEGMENT`; the production owner was not checked.

## Design implications

- D5a / ADR-0165 mechanism unchanged; ADR-0165's untested "NICs with UDP segmentation offload" item is tested for the non-`UDP_SEGMENT` path.
- New constraint to pin. Options: (1) the guest-flow owner's host UDP sockets never set `UDP_SEGMENT` (option or control message), with a DELIVER test on the production owner path — smallest; (2) handle GSO skbs in the program — kernel helper refuses, `FIXED_GSO` did not fix it; (3) pass rather than drop an un-unframeable GSO skb — still wrong bytes, does not replace (1).
- V-16e stays open (hardware USO).

## Not proven

Hardware UDP segmentation; 6.18; IPv6; multiple egress interfaces (V-17); a second physical receiver; cause of the post-program loss in the `FIXED_GSO` experiment.

**Gate recommendation:** PROMOTE V-16a–c; pin the `UDP_SEGMENT` constraint.
