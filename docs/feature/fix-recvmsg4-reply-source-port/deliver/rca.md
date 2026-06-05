# RCA — recvmsg4 reply-source rewrite drops the source PORT

**Bug ID:** fix-recvmsg4-reply-source-port · **Feature arc:** GH #200 (unconnected-udp-sendmsg4) · **ADR:** ADR-0053 rev 2026-06-05 §D4 · **Paradigm:** OOP (@nw-software-crafter)

## Problem

For a same-host unconnected-UDP service whose VIP port differs from the backend
port (canonical: DNS `VIP:53 → backend:5353`), `cgroup_recvmsg4_service` restores
only the reply source **address** (`user_ip4 ← VIP`) and leaves the source **port**
as the backend's listening port. The client's `recvfrom` sees `(VIP_IP, backend_port)`
instead of `(VIP_IP, VIP_PORT)`. Source-validating resolvers (Unbound, BIND 9)
discard the reply, silently breaking the service.

## Verdict: genuine defect (NOT a documented same-port constraint)

- ADR-0053 §D4 (user-locked) directs recvmsg4 to *"write the VIP into `user_ip4` /
  `user_port`"* — names **both** fields. Implementation writes only `user_ip4`.
- Intra-ADR contradiction: D1 value schema is IP-only (`u32`) but D4 rewrite is
  addr+port. Unreconciled; the crafter followed D1.
- Forward path is full `(addr,port)` NAT by construction (`LocalBackendEntry`
  carries `backend_ip_host` + `backend_port_host`; sendmsg4/connect4 write both
  `user_ip4` + `user_port`). Cross-port is a *supported, deliberate* forward shape.
- Cilium `__sock4_xlate_rev` (the cited north star) stores AND restores both
  address and port. Current code is strictly weaker than its reference.
- No same-port scoping exists anywhere in the ADR.

## Root cause chain

**Branch A — production:** reverse map value omits the VIP port.
- `REVERSE_LOCAL_MAP: HashMap<ReverseLocalKey, u32>` — value is bare `u32`
  (`crates/overdrive-bpf/src/maps/reverse_local_map.rs:71`).
- `recvmsg4` has no VIP port to write back; rewrites only `user_ip4`
  (`crates/overdrive-bpf/src/programs/cgroup_recvmsg4_service.rs:156-158`,
  comment `:151-155` deliberately leaves the port).
- Root: reverse store schema specified IP-only, contradicting D4's addr+port
  directive.

**Branch B — why no gate caught it:** verification stack shares the IP-only model.
- Tier-3 `unconnected_udp_roundtrip.rs` asserts `src.ip()` only, never `src.port()`
  (`:221-227`, `:309`, `:324-327`, `:408-411`) — despite a cross-port fixture
  (VIP=53, backend forced off :53/:5353 at `:73`, `:196-197`).
- Sim `reply_mirror: BTreeMap<BackendKey, Ipv4Addr>` cannot represent a port
  (`crates/overdrive-sim/src/adapters/dataplane.rs:135`, `:199`); Tier-1 lockstep
  `reply_source_rewrite_lockstep.rs:51` asserts an IP only.
- No Tier-2 backstop (`BPF_PROG_TEST_RUN` ENOTSUPP for `cgroup_sock_addr` ≤ 6.8).

## Fix plan (all affected files)

1. **Kernel map value** `crates/overdrive-bpf/src/maps/reverse_local_map.rs`:
   `u32` → `ReverseLocalEntry { vip_host: u32, vip_port_host: u16, _pad: u16 }`
   (`#[repr(C)]`, 8-byte POD); add `assert!(size_of::<ReverseLocalEntry>() == 8)`.
   Key (`ReverseLocalKey`) unchanged.
2. **Kernel program** `crates/overdrive-bpf/src/programs/cgroup_recvmsg4_service.rs`:
   rewrite `user_port = u32::from(vip_port_host.to_be())` (low-16-NBO idiom — do
   NOT use `from_be(...) as u16`, the silent-0 trap; sendmsg4:90 is the precedent).
   Update the `:151-155` comment + module docstring `:57-59`.
3. **Userspace handle** `crates/overdrive-dataplane/src/maps/reverse_local_map_handle.rs`:
   new `ReverseLocalEntryPod` value type (`aya::Pod`, 8-byte assert); `upsert`/
   `entries`/codec gain `vip_port`; host-order roundtrip proptest extended to pin
   `vip_port_host` + new value byte layout.
4. **Dual-write call site** `crates/overdrive-dataplane/src/lib.rs`: pass `vip_port`
   into the reverse upsert (`~:2042`, already in scope from `register_local_backend`);
   update `probe_reverse_local` value shape (`~:1207-1251`) + map typing (`~:638-644`).
   **Reverse-first ordering preserved** — only the value payload widens.
5. **Sim mirror + invariant** `crates/overdrive-sim/src/adapters/dataplane.rs`:
   `reply_mirror` value `Ipv4Addr` → `SocketAddrV4`; `reply_source_for ->
   Option<SocketAddrV4>`; store `SocketAddrV4::new(vip, vip_port)`. Invariant
   `crates/overdrive-sim/src/invariants/reply_source_rewrite_lockstep.rs`: assert
   `(IP, port)`.
6. **Regression gate (RED → GREEN)**
   `crates/overdrive-dataplane/tests/integration/unconnected_udp_roundtrip.rs`:
   add `assert_eq!(src.port(), VIP_PORT, ...)` at every reply-source check. Audit
   `unconnected_udp_reply_hardening.rs` for the same gap.
7. **ADR-0053 D1/D4 reconciliation** — route through `@nw-solution-architect`
   (project rule: ADR edits go via the architect agent, never inline).

## Risk

Low. Value width 4→8 bytes is not a parity break (8-byte guard is on the *key*;
add a new *value* assert). No rkyv schema-evolution fixture needed — `REVERSE_LOCAL_MAP`
is a live BPF map recreated from intent on boot, not an rkyv/redb-persisted type.
The `user_port` NBO write is the high-risk micro-detail, guarded by the new Tier-3
port assertion (must be Lima-verified in the same PR). No safe interim mitigation
(VIP port isn't persisted to reconstruct); same-port deployments unaffected.
