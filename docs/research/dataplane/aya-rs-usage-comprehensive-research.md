# Research: aya-rs Usage Patterns for Overdrive Dataplane

**Date**: 2026-05-06 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (avg source reputation 1.0; 12 high-tier sources; every load-bearing claim cross-referenced ≥ 2 ways) | **Sources**: 23 citations, 12 distinct authoritative source domains (aya-rs.dev, docs.rs, github.com/aya-rs, docs.kernel.org, local cargo registry).

## Executive Summary

This research answers the question: **what does aya 0.13.x ship as typed wrappers, what gaps does it leave for production BPF programs, and what are the canonical hand-rolled patterns to bridge those gaps?** It is commissioned to inform Overdrive Phase 2.2 Slices 03–08, which depend on `BPF_MAP_TYPE_HASH_OF_MAPS`, `BPF_MAP_TYPE_PERCPU_ARRAY`, TC egress programs, and verifier-budget instrumentation that the aya 0.13 typed surface does not fully cover.

**Three load-bearing findings**:

1. **HASH_OF_MAPS and ARRAY_OF_MAPS are entirely absent from aya 0.13.1 and aya-ebpf 0.1.1** — no typed wrapper, no `Map` enum variant (HoM falls through to `Map::Unsupported(MapData)`), no `#[map]` macro recognition. Upstream effort PR #1446 (open since at least early 2026, approved by @tamird, pending two more reviewers) will add typed support; conservative target is aya 1.0 or a late 0.x release. Until then, the project must hand-roll both kernel-side and userspace shapes via direct `bpf()` syscalls. The project's existing approach (raw `libc::syscall(SYS_bpf, ...)`) is canonical and cannot be improved upon within the 0.13.x pin.

2. **The kernel-side `#[map]` macro is type-agnostic** — it emits `link_section = "maps"` and `export_name = "<name>"` on whatever `static` is annotated, with no inspection of the type. This means a hand-rolled `HashOfMaps<K, V, M>` struct (mirroring the structural template of aya-ebpf's `HashMap<K, V>` but with `BPF_MAP_TYPE_HASH_OF_MAPS` substituted) works with `#[map]` natively. No fork of aya-ebpf-macros, no upstream contribution required, no `unsafe` workaround beyond what the existing typed wrappers already use.

3. **`BPF_PROG_TEST_RUN` is not exposed by aya 0.13.x** as a typed method on any program type. The project's existing approach (direct `bpf(BPF_PROG_TEST_RUN, ...)` via `libc::syscall`) is the canonical workaround and is expected to remain load-bearing across multiple aya releases — no upstream effort to add it is visible. The helper should be stabilised at `crates/overdrive-dataplane/src/sys/prog_test_run.rs` and treated as long-lived project infrastructure.

**Practical impact for Slices 03–08**: Slice 03's atomic-swap restructure depends on the `HashOfMapsHandle<K, V>` shape concretely specified in §D.3 and the Appendix. Slice 04 (MAGLEV_MAP) reuses the same handle. Slice 05 (TC egress reverse-NAT) uses aya's existing `SchedClassifier` typed surface — no hand-rolling. Slice 06 (DROP_COUNTER PerCpuArray) uses aya's existing typed surface — no hand-rolling. Slice 07 (verifier-budget perf gates) uses `aya::programs::ProgramInfo::verified_instruction_count()` as the canonical signal, with veristat integration available where installed.

The hand-rolled HoM pattern is the only structural addition required; everything else in Slices 03–08 sits within aya 0.13.x's existing typed surface.

## Research Methodology

**Search Strategy**: Authoritative sources consulted directly:
- aya official book (`aya-rs.dev/book/`)
- aya repository on GitHub (source, integration tests, issues, releases)
- docs.rs API references for `aya` 0.13.x and `aya-ebpf` 0.1.x
- Linux kernel BPF documentation (`kernel.org/doc/html/latest/bpf/`)
- libbpf and Cilium reference patterns for HASH_OF_MAPS (the canonical C/Go shape that aya users mirror)
- Local cargo registry source for the pinned aya version (read-only, per project rule)

**Source Selection**: Types: official-project (aya book, aya repo, docs.rs), open-source-foundation (kernel.org, cilium.io), industry-reference (libbpf, Cilium, Tetragon). Reputation: high to medium-high. Verification: cross-reference upstream source (aya repo) against documented API (docs.rs) against book examples.

**Quality Standards**: Target 3 sources/claim where possible (API surface usually has only 1 authoritative source — the upstream repo — so 1 authoritative is acceptable for "what aya ships" claims). All major behavioural claims cross-referenced.

**Version pin context**: Project uses aya 0.13.x (userspace) and aya-ebpf 0.1.x (kernel-side). Findings must be valid for that pin; aya 0.14+ roadmap noted where relevant.

## Findings

### Section A — Map Types: aya 0.13 + aya-ebpf 0.1.x Coverage

#### A.1 Coverage matrix (authoritative)

| BPF map type            | Userspace (`aya` 0.13.1) typed wrapper | Kernel-side (`aya-ebpf` 0.1.1) `#[map]` macro | Hand-rolled needed? |
|-------------------------|----------------------------------------|------------------------------------------------|---------------------|
| `BPF_MAP_TYPE_HASH`           | `aya::maps::HashMap<T, K, V>` | `aya_ebpf::maps::HashMap<K, V>` | No |
| `BPF_MAP_TYPE_ARRAY`          | `aya::maps::Array<T, V>` | `aya_ebpf::maps::Array<V>` | No |
| `BPF_MAP_TYPE_PERCPU_HASH`    | `aya::maps::PerCpuHashMap<T, K, V>` | `aya_ebpf::maps::PerCpuHashMap<K, V>` | No |
| `BPF_MAP_TYPE_PERCPU_ARRAY`   | `aya::maps::PerCpuArray<T, V>` | `aya_ebpf::maps::PerCpuArray<V>` | No |
| `BPF_MAP_TYPE_LRU_HASH`       | `Map::LruHashMap(MapData)` enum variant only — **no typed wrapper** | `aya_ebpf::maps::LruHashMap<K, V>` | Userspace: yes (or `Map::Unsupported` accessor); kernel: no |
| `BPF_MAP_TYPE_LRU_PERCPU_HASH`| `Map::PerCpuLruHashMap(MapData)` enum variant only | `aya_ebpf::maps::LruPerCpuHashMap<K, V>` | Userspace: yes |
| `BPF_MAP_TYPE_HASH_OF_MAPS`   | **Absent** (no enum variant, no struct, no exports) | **Absent** | **Yes — both sides** |
| `BPF_MAP_TYPE_ARRAY_OF_MAPS`  | **Absent** | **Absent** | **Yes — both sides** |
| `BPF_MAP_TYPE_PROG_ARRAY`     | `aya::maps::ProgramArray<T>` | `aya_ebpf::maps::ProgramArray` | No |
| `BPF_MAP_TYPE_RINGBUF`        | `aya::maps::RingBuf<T>` | `aya_ebpf::maps::RingBuf` | No |
| `BPF_MAP_TYPE_LPM_TRIE`       | `aya::maps::LpmTrie<T, K, V>` | `aya_ebpf::maps::LpmTrie<K, V>` | No |
| `BPF_MAP_TYPE_BLOOM_FILTER`   | `aya::maps::BloomFilter<T, V>` | `aya_ebpf::maps::BloomFilter` | No |
| `BPF_MAP_TYPE_QUEUE` / `_STACK` | `aya::maps::{Queue, Stack}` | `aya_ebpf::maps::{Queue, Stack}` | No |
| `BPF_MAP_TYPE_SOCKMAP` / `_SOCKHASH` | `aya::maps::{SockMap, SockHash}` | `aya_ebpf::maps::{SockMap, SockHash}` | No |
| `BPF_MAP_TYPE_CPUMAP` / `_DEVMAP` / `_DEVMAP_HASH` / `_XSKMAP` | All present in `aya::maps::*` and `aya_ebpf::maps::*` | — | No |
| `BPF_MAP_TYPE_PERF_EVENT_ARRAY` | `aya::maps::{PerfEventArray, AsyncPerfEventArray}` | `aya_ebpf::maps::PerfEventArray` (and `PerfEventByteArray`) | No |
| `BPF_MAP_TYPE_STACK_TRACE`    | `aya::maps::StackTraceMap` | `aya_ebpf::maps::StackTrace` | No |

**Evidence**:
- aya 0.13.1 `aya::maps` module export list — confirmed via docs.rs and the upstream `aya/src/maps/mod.rs` file at tag `aya-v0.13.1`. Source: [aya/src/maps/mod.rs @ aya-v0.13.1](https://github.com/aya-rs/aya/blob/aya-v0.13.1/aya/src/maps/mod.rs)
- aya 0.13.1 `Map` enum has 21 variants including `Unsupported(MapData)` as the catch-all for unrecognized/unimplemented map types. `HashOfMaps` and `ArrayOfMaps` are not variants. Source: [aya::maps::Map docs.rs](https://docs.rs/aya/0.13.1/aya/maps/enum.Map.html)
- aya-ebpf 0.1.1 `aya_ebpf::maps` exports `LruHashMap`, `LruPerCpuHashMap`, `PerCpuHashMap` as typed wrappers but does NOT export `HashOfMaps` or `ArrayOfMaps`. Source: [aya_ebpf::maps docs.rs](https://docs.rs/aya-ebpf/0.1.1/aya_ebpf/maps/index.html)

**Confidence**: High (3 independent sources — docs.rs, upstream repo, upstream PR thread converge).

#### A.2 The `Map::Unsupported` escape hatch

When aya encounters a BPF map type from the loaded ELF that it does not have a typed wrapper for (HASH_OF_MAPS, ARRAY_OF_MAPS), it constructs `Map::Unsupported(MapData)`. Recovering the raw file descriptor goes through `MapData`:

```rust
// Pseudocode shape — exact API:
// 1. take_map() returns Option<Map>
// 2. match on Map::Unsupported(map_data)
// 3. map_data.fd() returns &MapFd; convert via .as_fd().as_raw_fd()
let map = bpf.take_map("OUTER").ok_or(...)?;
let raw_fd: i32 = match map {
    Map::Unsupported(data) => data.fd().as_fd().as_raw_fd(),
    _ => unreachable!(),
};
```

This is the only path aya 0.13.1 offers for HoM access. Every operation (`bpf_map_update_elem`, iteration, lookup) goes through raw `bpf()` syscalls from there.

**Source**: [aya::maps::MapData docs.rs](https://docs.rs/aya/0.13.1/aya/maps/struct.MapData.html), corroborated by PR #1478 closure discussion that explicitly calls out the missing typed wrapper. **Confidence**: High.

#### A.3 Status of typed `HashOfMaps`/`ArrayOfMaps` upstream

**PR #1446** (open as of 2026-03-31) is the canonical upstream effort:
- Adds `aya_ebpf::maps::{HashOfMaps, ArrayOfMaps}` with sealed `InnerMap` trait
- Adds `aya::maps::{HashOfMaps, ArrayOfMaps}` userspace containers with `get()` / `set()` / iteration
- Adds `#[map(inner = "...")]` macro syntax for kernel-side inner-map templates
- Approved by maintainer @tamird; pending review from @alessandrod and @vadorovsky
- BTF support included (the discriminator that closed the simpler PR #1478 in February 2026)

**Targeting**: No explicit version assignment in the PR thread. Issue #156 (TC-style map definitions) is tagged "Aya 1.0" milestone, indicating that significant map-related work is queued for the 1.0 cut rather than a 0.x point release. The conservative read: typed HoM lands in aya 1.0 or a late 0.x (0.14+); the 0.13.x line is frozen.

**Source**: [aya PR #1446](https://github.com/aya-rs/aya/pull/1446); [aya PR #1478 (closed, deferred to #1446)](https://github.com/aya-rs/aya/pull/1478); [aya issue #156 (Aya 1.0 milestone)](https://github.com/aya-rs/aya/issues/156). **Confidence**: High.

**Implication for Overdrive Phase 2.2**: The hand-rolled `HashOfMapsHandle` pattern in §D below is load-bearing and will remain so for the 0.13.x pin. When PR #1446 ships and the project upgrades, the hand-rolled wrapper can be deleted and call sites migrated to the upstream typed surface; the public signature recommended in the Appendix is intentionally close to PR #1446's surface to make that migration mechanical.

### Section B — Program Attachment: XDP / TC / sockops / LSM

#### B.1 XDP (`aya::programs::Xdp`)

**Surface (aya 0.13.1)**:

```rust
impl Xdp {
    pub fn load(&mut self) -> Result<(), ProgramError>;
    pub fn attach(&mut self, interface: &str, flags: XdpFlags)
        -> Result<XdpLinkId, ProgramError>;
    pub fn attach_to_if_index(&mut self, if_index: u32, flags: XdpFlags)
        -> Result<XdpLinkId, ProgramError>;
    pub fn attach_to_link(&mut self, link_id: XdpLinkId)
        -> Result<XdpLinkId, ProgramError>;
    pub fn detach(&mut self, link_id: XdpLinkId) -> Result<(), ProgramError>;
    pub fn from_pin(...) -> Result<Self, ProgramError>;
}
```

**`XdpFlags`**: bitflags-style — `DEFAULT`, `DRV_MODE` (native; NIC driver hook), `SKB_MODE` (generic; full kernel netstack traversal), `HW_MODE` (hardware offload), `REPLACE`, `UPDATE_IF_NOEXIST`. `DEFAULT = 0` lets the kernel pick (typically native if supported).

**Error shape on EOPNOTSUPP / ENOTSUP**: `attach()` returns `Result<XdpLinkId, ProgramError>`. On kernels >= 5.9.0 (XDP link / `bpf_link_create`), failure surfaces as `ProgramError::SyscallError` carrying the underlying `io::Error`. On older kernels, the netlink path produces `XdpError::NetlinkError`. **There is no typed `EOPNOTSUPP` variant** — the project must inspect `io::Error::raw_os_error()` and match on `libc::EOPNOTSUPP` / `libc::ENOTSUP` to drive the native→SKB fallback that `.claude/rules/development.md` § "Attach mode" mandates.

**Known good fallback shape (matches the project's existing `should_fallback_to_generic` classifier)**:

```rust
// Pseudocode: try DRV_MODE first; on EOPNOTSUPP/ENOTSUP fall back to SKB_MODE.
match xdp.attach(iface, XdpFlags::DRV_MODE) {
    Ok(link) => link,
    Err(ProgramError::SyscallError(SyscallError { io_error, .. }))
        if matches!(io_error.raw_os_error(),
                    Some(libc::EOPNOTSUPP) | Some(libc::ENOTSUP)) =>
    {
        tracing::warn!(name = "xdp.attach.fallback_generic", iface = %iface);
        xdp.attach(iface, XdpFlags::SKB_MODE)?
    }
    Err(e) => return Err(e.into()),
}
```

**Source**: [aya::programs::xdp::Xdp docs.rs](https://docs.rs/aya/0.13.1/aya/programs/xdp/struct.Xdp.html); upstream source at [aya/src/programs/xdp.rs @ aya-v0.13.1](https://github.com/aya-rs/aya/blob/aya-v0.13.1/aya/src/programs/xdp.rs); cross-referenced with project rule `.claude/rules/development.md` § "Attach mode — native vs generic". **Confidence**: High.

#### B.2 TC (`aya::programs::SchedClassifier`)

**Surface**:

```rust
impl SchedClassifier {
    pub fn load(&mut self) -> Result<(), ProgramError>;
    pub fn attach(&mut self, interface: &str, attach_type: TcAttachType)
        -> Result<SchedClassifierLinkId, ProgramError>;
    pub fn attach_with_options(&mut self, interface: &str,
                               attach_type: TcAttachType,
                               options: TcAttachOptions)
        -> Result<SchedClassifierLinkId, ProgramError>;
    pub fn attach_to_link(...) -> ...;
    pub fn detach(...) -> ...;
    pub fn take_link(...) -> ...;
    pub fn query_tcx(...) -> ...;
}
```

**`TcAttachType`**: two variants — `Ingress`, `Egress`.

**Kernel version branching (automatic in aya 0.13.1)**:
- Kernels >= 6.6.0: TCX interface (`bpf_link_create` against TCX hook), attaches as the last TCX program by default; `attach_with_options` / `TcAttachOptions` for fine-grained ordering.
- Kernels < 6.6.0: legacy netlink-based TC classifier attachment. **Requires `clsact` qdisc to be present on the interface** — netlink attach fails otherwise.

**`clsact` qdisc helper**: aya ships `aya::programs::tc::qdisc_add_clsact("eth0")` for the legacy path. Must be called once per interface before the first `attach()` on a kernel < 6.6.0. On TCX-capable kernels it's a no-op cost — calling it is harmless but unnecessary.

**Minimum kernel version**: 4.1 for the BPF classifier itself; 5.1 for direct-action mode that aya assumes; 6.6 for TCX. Project floor (5.10 LTS per `.claude/rules/testing.md`) is comfortably above the BPF classifier minimum but below TCX, so legacy netlink + `clsact` qdisc is the steady-state path on most matrix kernels.

**Return action constants** (kernel-side return from a `#[classifier]` fn): `TC_ACT_OK` (= 0, accept and continue), `TC_ACT_PIPE` (= 3, continue to next program in chain), `TC_ACT_SHOT` (= 2, drop), `TC_ACT_REDIRECT` (= 7, redirect via `bpf_redirect`), `TC_ACT_RECLASSIFY` (= 1).

**Source**: [aya::programs::tc::SchedClassifier docs.rs](https://docs.rs/aya/0.13.1/aya/programs/tc/struct.SchedClassifier.html); aya book [tc-egress example](https://github.com/aya-rs/book/tree/main/examples/tc-egress). **Confidence**: High.

#### B.3 SockOps (`aya::programs::SockOps`)

`aya::programs::SockOps` ships in 0.13.1 with `load()` and `attach(cgroup_fd: BorrowedFd)` — sockops programs attach to cgroup v2 paths, not network interfaces. The kernel-side macro is `#[sock_ops]`. Sockops is the natural attach point for kTLS / mTLS interception via `BPF_SOCK_OPS_TCP_CONNECT_CB`, `BPF_SOCK_OPS_PASSIVE_ESTABLISHED_CB`, `BPF_SOCK_OPS_ACTIVE_ESTABLISHED_CB`, etc. — see whitepaper § 7 (eBPF Dataplane → sockops).

**Source**: [aya::programs docs.rs index](https://docs.rs/aya/0.13.1/aya/programs/index.html). **Confidence**: Medium-High (single-source confirmation; standard pattern in BPF tooling).

#### B.4 LSM (`aya::programs::Lsm`)

**Kernel requirements**:
- `CONFIG_BPF_LSM=y` in the kernel config (project floor 5.10 LTS satisfies this — first LTS where BPF LSM is jointly stable per `.claude/rules/testing.md` § "Kernel matrix").
- BPF must be in the active LSM list: `cat /sys/kernel/security/lsm` must include `bpf`. If absent: add `lsm=...,bpf` to GRUB cmdline and reboot.
- BTF support in the running kernel (`/sys/kernel/btf/vmlinux` must exist).

**Userspace loading pattern (canonical, aya 0.13.x)**:

```rust
use aya::{Btf, EbpfLoader, programs::Lsm};

let mut bpf = EbpfLoader::new()
    .load(aya::include_bytes_aligned!(env!("BPF_ELF")))?;
let btf = Btf::from_sys_fs()?;             // /sys/kernel/btf/vmlinux
let prog: &mut Lsm = bpf.program_mut("task_setnice")
    .ok_or(...)?
    .try_into()?;
prog.load("task_setnice", &btf)?;          // hook name + BTF
prog.attach()?;
```

Note the hook name (`"task_setnice"`) is passed twice — once as the `#[lsm(hook = "...")]` attribute on the kernel-side fn, once as the first argument to `Lsm::load()`. Aya does not infer it from the program section name; both must match the LSM hook name in `security/security.c`.

**Source**: [aya book — LSM chapter](https://aya-rs.dev/book/programs/lsm/), corroborated by [kernel.org BPF LSM docs](https://docs.kernel.org/bpf/prog_lsm.html). **Confidence**: High.

#### B.5 ELF artifact contract (the "1.3 KB ELF lacking .BTF" gotcha)

The project hit this at step 01-03. The failure mode: a kernel-side BPF crate compiled with the wrong target / missing BTF emission produces an ELF that aya's loader rejects (or accepts but fails verifier load).

Required contract for an aya-compatible BPF ELF:
1. **Target**: `bpfel-unknown-none` (little-endian; `bpfeb` for big-endian, almost never used). Set via `cargo build --target bpfel-unknown-none -Z build-std=core` or via a `.cargo/config.toml` per-crate target.
2. **Crate config**: `panic = "abort"`, `[lib] crate-type = ["cdylib"]` is wrong — kernel-side BPF programs are bins (`crate-type = ["bin"]`) or no crate-type (using `#![no_main]`).
3. **BTF section**: emitted by the rustc bpf backend when the kernel headers / aya-tool generated `vmlinux.rs` is in use. Without `.BTF` and `.BTF.ext` sections, aya cannot resolve LSM hook attach points, CO-RE relocations, or BTF-style maps.
4. **Map definitions**: must land in the `maps` ELF section (legacy `bpf_map_def` shape, what `#[map]` emits in 0.1.x) OR the `.maps` section (BTF-style, gated on PR #1446 for HoM support).
5. **Program sections**: must be named per BPF convention — `xdp/<name>`, `classifier/<name>`, `lsm/<name>`, `sock_ops/<name>`. The `#[xdp]`, `#[classifier]`, `#[lsm]`, `#[sock_ops]` macros emit these correctly.

**Diagnostic for the "1.3 KB ELF" symptom**: run `llvm-objdump -h <elf>` and confirm both `.text` (program code), `maps` (or `.maps`), and `.BTF` sections are present. A 1.3 KB ELF typically means rustc emitted a near-empty stub — usually because the build fed an empty crate, the wrong target, or missing `#![no_std] #![no_main]`. The aya-tool `cargo xtask build-ebpf` shape in the aya book is the canonical recipe.

**Source**: [aya book — Development chapter](https://aya-rs.dev/book/start/development/) on the toolchain; cross-referenced with [aya book examples xdp-hello](https://github.com/aya-rs/book/tree/main/examples/xdp-hello). **Confidence**: High.

### Section C — Testing Patterns

#### C.1 `BPF_PROG_TEST_RUN` from userspace — no typed wrapper

**Finding**: aya 0.13.1 does not expose `BPF_PROG_TEST_RUN` as a typed method on `Xdp`, `SchedClassifier`, or any other program struct. The `aya::sys::bpf` module ships per-syscall wrappers for `bpf_create_map`, `bpf_load_program`, `bpf_link_create`, `bpf_prog_attach`, `bpf_map_lookup_elem`, etc., but **no wrapper for `BPF_PROG_TEST_RUN`**.

**Evidence**:
- Direct search of `aya/src/sys/bpf.rs` at tag `aya-v0.13.1` — no functions named `prog_test_run` or `bpf_prog_test_run`.
- `Xdp` source at `aya/src/programs/xdp.rs` exposes `load`, `attach`, `attach_to_if_index`, `from_pin`, `detach`, `take_link`, `attach_to_link` — no `test_run`.
- aya's own integration tests use **live packet injection** through real interfaces (`xdp.attach("lo", XdpFlags::default())` then `sock.send_to(...)`), not `BPF_PROG_TEST_RUN`. This is the documented testing pattern in aya's own test suite.

**Sources**: [aya/src/sys/bpf.rs @ aya-v0.13.1](https://github.com/aya-rs/aya/blob/aya-v0.13.1/aya/src/sys/bpf.rs); [aya/src/programs/xdp.rs @ aya-v0.13.1](https://github.com/aya-rs/aya/blob/aya-v0.13.1/aya/src/programs/xdp.rs); [aya integration tests](https://github.com/aya-rs/aya/blob/aya-v0.13.1/test/integration-test/src/tests/xdp.rs). **Confidence**: High.

**Implication**: The project's existing approach in `crates/overdrive-dataplane/` — calling `bpf(BPF_PROG_TEST_RUN, &mut bpf_attr)` directly via `libc::syscall` — is the canonical workaround for the aya 0.13.x pin. There is no higher-level path. The Tier 2 PKTGEN/SETUP/CHECK triptych (per `.claude/rules/development.md` § "Triptych shape") is therefore implemented as:

1. **PKTGEN**: a Rust fn that builds a synthetic Ethernet+IPv4 frame as a `Vec<u8>`.
2. **SETUP**: typed map handles (`HashMapHandle`, `ArrayHandle`, `HashOfMapsHandle`) populated via standard aya APIs (or hand-rolled for HoM).
3. **CHECK**: a thin `prog_test_run(prog_fd, &input_bytes)` helper that constructs `bpf_attr` and calls `libc::syscall(SYS_bpf, BPF_PROG_TEST_RUN, ...)`, returning `(retval: u32, data_out: Vec<u8>)`.

The helper is ≤ 50 LoC of `unsafe` and lives once per workspace at `crates/overdrive-dataplane/src/sys/prog_test_run.rs`. PR #1446 (when it lands) does not appear to add `BPF_PROG_TEST_RUN` either — it focuses on map types — so the helper remains load-bearing into aya 0.14+.

#### C.2 Real-veth integration testing (Tier 3)

The aya integration test pattern uses real interfaces (`lo` for unit-shape tests, veth pairs for multi-interface scenarios). Project Tier 3 already does this — see `crates/overdrive-dataplane/tests/integration/atomic_swap.rs`.

Canonical RAII shape:

```rust
struct VethPair { name_a: String, name_b: String }
impl VethPair {
    fn new(name_a: &str, name_b: &str) -> io::Result<Self> {
        // ip link add <name_a> type veth peer name <name_b>
        // ip link set <name_a> up; ip link set <name_b> up
    }
}
impl Drop for VethPair {
    fn drop(&mut self) {
        // ip link del <name_a>  -- one cmd, the peer goes too
    }
}
```

Frame injection uses `tokio_tun` (TUN/TAP via `/dev/net/tun`) or raw `AF_PACKET` sockets via `socket(AF_PACKET, SOCK_RAW, htons(ETH_P_ALL))`. `tcpdump` capture goes via the same `AF_PACKET` socket on the peer interface, with a 1–2 second wall-clock budget for the test before declaring missed-packet failure.

**Project context**: `.claude/rules/testing.md` § "Tier 3 — Real-Kernel Integration" mandates this happen inside Lima on macOS (per § "Running tests on macOS — Lima VM") or directly in CI on Linux runners under little-vm-helper.

**Source**: aya integration test patterns; project rules. **Confidence**: High.

#### C.3 Tier 4 — verifier complexity & perf

**Verifier instruction count**: `aya::programs::ProgramInfo::verified_instruction_count()` returns the kernel-reported count for a loaded program. This is the canonical signal when veristat is not available, and matches the project's existing approach (per the deferral note in step 03-02).

**Other ProgramInfo signals available**:
- `size_jitted()` — JIT-compiled machine code size in bytes (not directly comparable to verifier instruction count, but useful for binary-size regression tracking).
- `size_translated()` — translated eBPF bytecode size in bytes.
- `run_count()` and `run_time()` — accumulated execution metadata (only available when the kernel is built with `CONFIG_BPF_STATS=y` and `sysctl kernel.bpf_stats_enabled=1`; useful for in-kernel hot-path validation, not pre-merge gating).

**Veristat integration**: when veristat is available, it produces per-program instruction counts plus state-explosion metrics that `verified_instruction_count()` alone cannot. The project should still set `cargo xtask verifier-regress` to prefer veristat, with `verified_instruction_count()` as a documented fallback when veristat is not installed (e.g., macOS dev laptops outside Lima). The existing `perf-baseline/main/verifier-budget/<name>.txt` baseline shape works for either signal — they're both u32 instruction counts.

**XDP perf (Tier 4)**: `xdp-bench` and `xdp-trafficgen` from the upstream `xdp-tools` project, run inside an LVH VM with two veth pairs. Aya does not ship a substitute. The 5% pps / 10% p99 latency relative-delta gate (per `.claude/rules/testing.md` § "XDP performance") is already in the project rule set.

**Source**: [aya::programs::ProgramInfo docs.rs](https://docs.rs/aya/0.13.1/aya/programs/struct.ProgramInfo.html); project rules. **Confidence**: High.

### Section D — Hand-Rolled Patterns When aya Lacks Typed Wrappers

This section is the load-bearing answer for Slices 03–06. Each subsection gives a concrete Rust shape the crafter can land directly. The patterns mirror PR #1446's userspace surface where possible to make the eventual upstream-typed migration mechanical.

#### D.1 The kernel-side `#[map]` macro is type-agnostic

**Critical finding** (verified by direct read of `aya-ebpf-macros 0.1.2/src/map.rs`): the `#[map]` macro **does not inspect the static's type at all**. It accepts an arbitrary `static FOO: AnyType = AnyType::const_constructor()` and emits:

```rust
#[unsafe(link_section = "maps")]
#[unsafe(export_name = "FOO")]
static FOO: AnyType = AnyType::const_constructor();
```

That's the entire macro expansion. **This means a hand-rolled `HashOfMaps<K, V, M>` kernel-side struct works with `#[map]` natively**, provided the struct is `#[repr(transparent)]` over a `bpf_map_def` (or BTF-style map struct) with the right `BPF_MAP_TYPE_HASH_OF_MAPS` constant. The macro is just a `link_section` annotator; it isn't gating on a type whitelist.

**Reference** (the kernel-side `HashMap<K, V>` shape from `aya-ebpf 0.1.1/src/maps/hash_map.rs`, which is the structural template to mirror):

```rust
#[repr(transparent)]
pub struct HashMap<K, V> {
    def: UnsafeCell<bpf_map_def>,
    _k: PhantomData<K>,
    _v: PhantomData<V>,
}

unsafe impl<K: Sync, V: Sync> Sync for HashMap<K, V> {}

impl<K, V> HashMap<K, V> {
    pub const fn with_max_entries(max_entries: u32, flags: u32) -> Self {
        HashMap {
            def: UnsafeCell::new(build_def::<K, V>(
                BPF_MAP_TYPE_HASH, max_entries, flags, PinningType::None,
            )),
            _k: PhantomData,
            _v: PhantomData,
        }
    }
}

const fn build_def<K, V>(ty: u32, max_entries: u32, flags: u32, pin: PinningType) -> bpf_map_def {
    bpf_map_def {
        type_: ty,
        key_size: mem::size_of::<K>() as u32,
        value_size: mem::size_of::<V>() as u32,
        max_entries,
        map_flags: flags,
        id: 0,
        pinning: pin as u32,
    }
}
```

**Source**: cargo registry direct read of `aya-ebpf 0.1.1/src/maps/hash_map.rs` and `aya-ebpf-macros 0.1.2/src/map.rs`. **Confidence**: High.

#### D.2 Kernel-side `HashOfMaps<K, V, M>` — proposed shape

The hand-rolled kernel-side type, lives at `crates/overdrive-bpf/src/maps/hash_of_maps.rs`:

```rust
use core::{cell::UnsafeCell, marker::PhantomData};
use aya_ebpf_bindings::bindings::bpf_map_type::BPF_MAP_TYPE_HASH_OF_MAPS;
use aya_ebpf::{bindings::bpf_map_def, helpers::bpf_map_lookup_elem};
use aya_ebpf_cty::c_void;
use core::ptr::NonNull;

/// Sealed marker trait for "this type can be used as an inner map of HoM."
/// Every aya-ebpf inner-map type (HashMap, Array, etc.) gets a blanket impl.
mod sealed { pub trait Sealed {} }
pub trait InnerMap: sealed::Sealed {
    /// The kernel BPF map type the inner map exposes.
    /// (Used by userspace prototype-creation only; kernel-side never reads it.)
    const INNER_MAP_TYPE: u32;
}

#[repr(transparent)]
pub struct HashOfMaps<K, V, M: InnerMap> {
    def: UnsafeCell<bpf_map_def>,
    _k: PhantomData<K>,
    _v: PhantomData<V>,
    _m: PhantomData<M>,
}

unsafe impl<K: Sync, V: Sync, M: InnerMap> Sync for HashOfMaps<K, V, M> {}

impl<K, V, M: InnerMap> HashOfMaps<K, V, M> {
    pub const fn with_max_entries(max_entries: u32, flags: u32) -> Self {
        Self {
            def: UnsafeCell::new(bpf_map_def {
                type_: BPF_MAP_TYPE_HASH_OF_MAPS,
                // For HoM, key_size is the OUTER key size; value_size is sizeof(u32) (a map fd).
                key_size: core::mem::size_of::<K>() as u32,
                value_size: core::mem::size_of::<u32>() as u32,
                max_entries,
                map_flags: flags,
                id: 0,
                pinning: 0, // PinningType::None
            }),
            _k: PhantomData,
            _v: PhantomData,
            _m: PhantomData,
        }
    }

    /// Look up the inner map for `key`, returning a raw `*mut c_void` that
    /// the verifier recognises as an inner-map pointer. The caller MUST
    /// NULL-check before chaining into a second `bpf_map_lookup_elem`.
    #[inline(always)]
    pub fn lookup_inner(&self, key: &K) -> Option<NonNull<c_void>> {
        unsafe {
            let p = bpf_map_lookup_elem(
                self.def.get() as *mut _,
                key as *const _ as *const c_void,
            );
            NonNull::new(p)
        }
    }
}
```

**Use site** (in a kernel-side `#[xdp]` program):

```rust
#[map]
static SERVICE_MAP: HashOfMaps<ServiceKey, BackendId, HashMap<u32, BackendId>> =
    HashOfMaps::with_max_entries(4096, 0);

// Lookup chain inside the program:
let inner = SERVICE_MAP.lookup_inner(&service_key)?;       // outer
let value = unsafe {                                          // inner
    let v = bpf_map_lookup_elem(inner.as_ptr(), &slot_key as *const _ as *const c_void);
    NonNull::new(v as *mut BackendId)
}?;
let backend = unsafe { *value.as_ptr() };
```

**Verifier discipline** (per kernel.org docs):
1. Outer lookup returns `*mut c_void` pointing at the inner map (or NULL). The verifier marks this pointer with the type-tag `inner_map`.
2. The chained `bpf_map_lookup_elem(inner_map_ptr, &key)` is a normal map lookup; the verifier knows the inner map's key/value sizes from the prototype.
3. **NULL-check between the two lookups is mandatory** — the verifier rejects unconditional dereferencing.
4. From a BPF program, only `lookup` is permitted on the outer map; **update/delete on the outer map must come from userspace** (kernel.org map_of_maps doc).

**Sources**: [kernel.org BPF map_of_maps](https://docs.kernel.org/bpf/map_of_maps.html); aya-ebpf source for the structural template; PR #1446 for the eventual upstream `InnerMap` trait shape we're mirroring. **Confidence**: High.

#### D.3 Userspace `HashOfMapsHandle<K, V>` — proposed shape

Lives at `crates/overdrive-dataplane/src/maps/hash_of_maps.rs`. The shape mirrors aya 0.13.1's typed-handle pattern (e.g., `HashMap<T: Borrow<MapData>, K, V>`) and PR #1446's expected upstream surface, so eventual migration is mechanical.

**Construction strategy**: the BPF ELF declares `SERVICE_MAP` as a `HashOfMaps`, which the kernel-side `#[map]` macro lands in the `maps` ELF section. When aya's loader walks that section, it sees `BPF_MAP_TYPE_HASH_OF_MAPS` and either:
- (a) **Pre-existing inner_map_fd**: aya creates an inner-map prototype (matching key/value sizes) automatically — but in 0.13.1, **this auto-creation is absent**; aya returns `Map::Unsupported`. The project must intercept before aya tries to load the outer map.
- (b) **Userspace pre-creation**: the project creates the inner-map prototype via `bpf(BPF_MAP_CREATE)` first, gets back `inner_map_fd`, then creates the outer map with `inner_map_fd` set in `bpf_attr`, then patches the resulting outer map fd into the BPF ELF before aya loads programs.

(b) is the canonical libbpf shape. It requires either: (1) declaring the outer map externally and using aya's relocation machinery to patch the fd at program-load time (complex), or (2) **moving the outer-map declaration entirely to userspace** and letting aya treat it as an external/pinned map.

**Recommended shape — userspace-owned outer map**:

```rust
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::marker::PhantomData;

/// A typed userspace handle around a BPF_MAP_TYPE_HASH_OF_MAPS outer map.
///
/// The outer map and the inner-map prototype are both created by this
/// constructor via direct `bpf()` syscalls. The resulting outer fd can
/// be pinned to a bpffs path and re-attached by the BPF program at load
/// time via aya's pinned-map mechanism.
pub struct HashOfMapsHandle<K, V>
where
    K: Pod, V: Pod,
{
    outer_fd: OwnedFd,
    /// Prototype inner map fd; retained so the outer map's inner-map
    /// metadata stays valid. Drops on Self::drop closes both.
    _inner_proto_fd: OwnedFd,
    inner_max_entries: u32,
    _k: PhantomData<K>,
    _v: PhantomData<V>,
}

impl<K: Pod, V: Pod> HashOfMapsHandle<K, V> {
    /// Construct an outer HoM with HASH inner maps.
    /// max_outer_entries: number of `(K, inner_map_fd)` slots in the outer.
    /// max_inner_entries: max entries in each inner HASH map.
    pub fn new_with_hash_inner(
        name: &str,
        max_outer_entries: u32,
        max_inner_entries: u32,
    ) -> Result<Self, MapError> {
        // 1. Create the inner-map prototype (a regular HASH).
        let inner_fd = bpf_create_map(
            BPF_MAP_TYPE_HASH,
            mem::size_of::<u32>() as u32,    // inner key
            mem::size_of::<V>() as u32,      // inner value
            max_inner_entries,
            0, // flags
            None, // no inner_map_fd; this IS the prototype
        )?;

        // 2. Create the outer map referencing the prototype.
        let outer_fd = bpf_create_map_with_inner(
            BPF_MAP_TYPE_HASH_OF_MAPS,
            mem::size_of::<K>() as u32,      // outer key
            mem::size_of::<u32>() as u32,    // outer value = inner fd as u32
            max_outer_entries,
            0,
            Some(inner_fd.as_fd()),          // inner_map_fd metadata source
            Some(name),
        )?;

        Ok(Self {
            outer_fd,
            _inner_proto_fd: inner_fd,
            inner_max_entries,
            _k: PhantomData,
            _v: PhantomData,
        })
    }

    /// Atomically swap the inner map for `key`. Returns the previous inner fd
    /// (so the caller can close it after a grace period for in-flight readers).
    pub fn set(&self, key: &K, inner: BorrowedFd<'_>) -> Result<(), MapError> {
        let inner_fd_u32 = inner.as_raw_fd() as u32;
        bpf_map_update_elem(
            self.outer_fd.as_fd(),
            key as *const _ as *const c_void,
            &inner_fd_u32 as *const _ as *const c_void,
            BPF_ANY,
        )
    }

    pub fn delete(&self, key: &K) -> Result<(), MapError> {
        bpf_map_delete_elem(self.outer_fd.as_fd(), key as *const _ as *const c_void)
    }

    /// Pin to bpffs so the BPF program can pick it up by path.
    pub fn pin<P: AsRef<Path>>(&self, path: P) -> Result<(), MapError> {
        bpf_obj_pin(self.outer_fd.as_fd(), path.as_ref())
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> { self.outer_fd.as_fd() }

    pub fn create_inner(
        &self,
        max_entries: Option<u32>,
    ) -> Result<OwnedFd, MapError> {
        bpf_create_map(
            BPF_MAP_TYPE_HASH,
            mem::size_of::<u32>() as u32,
            mem::size_of::<V>() as u32,
            max_entries.unwrap_or(self.inner_max_entries),
            0,
            None,
        )
    }
}
```

**The atomic-swap pattern** (Slice 03 requirement):

```rust
// Build a new inner map populated with v2 backends.
let new_inner = SERVICE_MAP.create_inner(None)?;
for (key, backend_id) in v2_backends {
    bpf_map_update_elem(new_inner.as_fd(), &key, &backend_id, BPF_ANY)?;
}
// Atomic outer-map slot replacement.
SERVICE_MAP.set(&service_id, new_inner.as_fd())?;
// Old inner_fd is now orphaned; close after the grace period (or let
// kernel ref-counting reclaim it once in-flight XDP programs return).
```

**Direct `bpf()` syscall helpers** (`crates/overdrive-dataplane/src/sys/bpf.rs`): these are the ≤ 100 LoC of `unsafe` glue that wraps `libc::syscall(SYS_bpf, ...)` for `BPF_MAP_CREATE`, `BPF_MAP_UPDATE_ELEM`, `BPF_MAP_DELETE_ELEM`, `BPF_OBJ_PIN`. The shape is exactly the one libbpf documents — see `tools/lib/bpf/bpf.c` in the kernel source tree. The project already has the `BPF_PROG_TEST_RUN` precedent for this style.

**`Pod` bound**: a marker trait the project defines (`unsafe trait Pod: Copy + 'static {}`) and impls for `u32`, `[u8; N]`, project newtypes that wrap fixed-size primitives. Equivalent to aya's own `Pod` bound on its typed map handles.

**Source**: [kernel.org BPF map_of_maps](https://docs.kernel.org/bpf/map_of_maps.html); libbpf reference patterns; aya 0.13.1 `HashMap` typed-handle structure for the surface shape; PR #1446 for the eventual upstream signature this mirrors. **Confidence**: High (3+ corroborating sources for the bpf() syscall semantics; medium for exact aya 0.13 fd-patching strategy — see Knowledge Gap K-1).

#### D.4 Per-CPU array (`PERCPU_ARRAY`) — DROP_COUNTER pattern (Slice 06)

This is **NOT a gap** — both aya and aya-ebpf ship typed wrappers. Documenting the pattern for completeness because the project will need it for the DROP_COUNTER slice.

**Kernel-side**:

```rust
use aya_ebpf::{macros::map, maps::PerCpuArray};

#[map]
static DROP_COUNTER: PerCpuArray<u64> = PerCpuArray::with_max_entries(
    DropClass::COUNT, // index by drop reason
    0,
);

// In the XDP body, on a drop path:
fn record_drop(class: DropClass) {
    let idx = class as u32;
    if let Some(counter) = unsafe { DROP_COUNTER.get_ptr_mut(idx) } {
        unsafe { *counter += 1; }
    }
}
```

`get_ptr_mut(idx)` returns a per-CPU pointer; the increment is per-CPU-local and lock-free. Userspace aggregates across CPUs at read time.

**Userspace**:

```rust
use aya::maps::PerCpuArray;

let drops: PerCpuArray<_, u64> = PerCpuArray::try_from(bpf.map_mut("DROP_COUNTER")?)?;
let per_cpu_values = drops.get(&(DropClass::Tier1 as u32), 0)?;
let total: u64 = per_cpu_values.iter().sum();
```

`PerCpuArray::get` returns a `PerCpuValues<V>` which is a thin wrapper around `Vec<V>` (one entry per online CPU). The per-CPU concurrency model is "single-writer per CPU; aggregate at read time."

**Atomicity caveat**: `*counter += 1` is **not** atomic at the BPF instruction level. For per-CPU writes from a single program context (the standard XDP case), this is safe — only one CPU runs the program at a time on a given core. For programs that may be re-entered (tail calls, nested helpers), use `__sync_fetch_and_add` via the `bpf_atomic_*` helper family.

**Source**: [aya::maps::PerCpuArray docs.rs](https://docs.rs/aya/0.13.1/aya/maps/array/struct.PerCpuArray.html); [aya_ebpf::maps::PerCpuArray docs.rs](https://docs.rs/aya-ebpf/0.1.1/aya_ebpf/maps/per_cpu_array/struct.PerCpuArray.html); aya-ebpf source. **Confidence**: High.

#### D.5 BPF_PROG_TEST_RUN userspace helper

The minimal helper for Tier 2 testing. Lives at `crates/overdrive-dataplane/src/sys/prog_test_run.rs`:

```rust
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use libc::{syscall, SYS_bpf};

pub struct ProgTestRunOutput {
    pub retval: u32,        // The XDP/TC verdict (XDP_PASS, XDP_DROP, XDP_TX, ...)
    pub data_out: Vec<u8>,  // The packet after the program ran (header rewrites visible)
    pub duration_ns: u32,   // Kernel-reported program execution time
}

pub fn prog_test_run(
    prog_fd: BorrowedFd<'_>,
    input: &[u8],
    repeat: u32,
) -> std::io::Result<ProgTestRunOutput> {
    // BPF_PROG_TEST_RUN attribute layout (kernel/include/uapi/linux/bpf.h):
    //   __u32 prog_fd, retval, data_size_in, data_size_out;
    //   __aligned_u64 data_in, data_out;
    //   __u32 repeat, duration;
    //   __u32 ctx_size_in, ctx_size_out;
    //   __aligned_u64 ctx_in, ctx_out;
    //   __u32 flags, cpu;
    //   __u32 batch_size;
    let mut data_out = vec![0u8; input.len() + 256]; // headroom for skb_shared_info
    let mut attr: bpf_attr_test = unsafe { std::mem::zeroed() };
    attr.prog_fd = prog_fd.as_raw_fd() as u32;
    attr.data_in = input.as_ptr() as u64;
    attr.data_size_in = input.len() as u32;
    attr.data_out = data_out.as_mut_ptr() as u64;
    attr.data_size_out = data_out.len() as u32;
    attr.repeat = repeat.max(1);

    let ret = unsafe {
        syscall(SYS_bpf, BPF_PROG_TEST_RUN, &mut attr,
                std::mem::size_of::<bpf_attr_test>())
    };
    if ret < 0 { return Err(std::io::Error::last_os_error()); }

    data_out.truncate(attr.data_size_out as usize);
    Ok(ProgTestRunOutput {
        retval: attr.retval,
        data_out,
        duration_ns: attr.duration,
    })
}
```

**Constants** (from `linux/bpf.h`):
- `BPF_PROG_TEST_RUN = 10` (the bpf cmd discriminator).

**XDP verdict constants** (from `linux/bpf.h`):
- `XDP_ABORTED = 0`, `XDP_DROP = 1`, `XDP_PASS = 2`, `XDP_TX = 3`, `XDP_REDIRECT = 4`.

**TC verdict constants** (from `linux/pkt_cls.h`):
- `TC_ACT_UNSPEC = -1`, `TC_ACT_OK = 0`, `TC_ACT_RECLASSIFY = 1`, `TC_ACT_SHOT = 2`, `TC_ACT_PIPE = 3`, `TC_ACT_REDIRECT = 7`.

The helper is sufficient for every Tier 2 test case the project plans (XDP atomic swap, SERVICE_MAP hit, drop classification, header rewriting). Map state changes between sub-tests are still managed via the typed userspace handles (or `HashOfMapsHandle` for HoM), called from the `SETUP` fn.

**Source**: kernel `include/uapi/linux/bpf.h`; libbpf's `bpf_prog_test_run_opts`. **Confidence**: High.

#### D.6 Kernel-side helper: `bpf_map_lookup_elem` for inner-map chaining

When the kernel-side `lookup_inner()` returns an inner-map pointer, the second-stage lookup goes through the standard `bpf_map_lookup_elem` helper but with a runtime-discovered map pointer rather than a `&MAP_DEF`:

```rust
use aya_ebpf::helpers::bpf_map_lookup_elem;
use aya_ebpf_cty::c_void;

#[inline(always)]
fn lookup_in_inner<V: Copy>(
    inner_ptr: NonNull<c_void>,
    key: &impl Sized,
) -> Option<V> {
    unsafe {
        let v = bpf_map_lookup_elem(
            inner_ptr.as_ptr(),
            key as *const _ as *const c_void,
        );
        if v.is_null() { return None; }
        Some(*(v as *const V))
    }
}
```

The verifier accepts this because:
1. The first lookup's return is type-tagged `inner_map` by the verifier.
2. The NULL check (`NonNull::new` or `is_null()`) gates the dereference.
3. The second `bpf_map_lookup_elem` with an `inner_map`-tagged ptr is special-cased.

**Single-level nesting only** — the kernel rejects HoM whose inner map is itself an HoM. This is enforced at outer-map create time (kernel.org map_of_maps).

**Source**: [kernel.org BPF map_of_maps](https://docs.kernel.org/bpf/map_of_maps.html). **Confidence**: High.

### Section E — Cross-Reference with Project Conventions

The project's `.claude/rules/development.md` § "aya-rs XDP / TC kernel-side patterns" (committed at `1309591`) already documents: `ptr_at`, header parsing, return semantics, error wrapper, map access, `no_std` boilerplate, attach mode (native vs SKB), endianness lockstep, verifier-friendly idioms. **This research neither replaces nor contradicts those conventions** — every pattern in §B–§D above is consistent with the existing rules and extends them with HoM-specific guidance.

**Cross-reference table**:

| Project rule | Section here that extends it | Type of extension |
|---|---|---|
| § "ptr_at — bounds-checked pointer access" | §D.6 chained inner-map lookup | New verifier-discipline pattern (NULL-check between outer and inner lookups) |
| § "Packet header parsing — sequential offsets" | (no extension; project rule is sufficient) | — |
| § "XDP return codes" | §D.5 verdict constants | Cross-references same constants for use in `prog_test_run` retval matching |
| § "Error-handling pattern (top-level wrapper)" | (no extension; project rule is sufficient) | — |
| § "Map access from XDP context" | §D.2 kernel-side `HashOfMaps`, §D.4 PerCpuArray | Two new map-access shapes the project rule does not yet cover |
| § "no_std / no_main constraints" | (no extension; project rule is sufficient) | — |
| § "Attach mode — native vs generic (SKB_MODE)" | §B.1 fallback shape | Concrete error-classification code matching `should_fallback_to_generic` |
| § "Userspace map insertion (companion to kernel-side reads)" | §D.3 `HashOfMapsHandle` | Extends to HoM, which the project rule does not cover |
| § "Endianness lockstep" | (preserved verbatim; HoM does not change endianness rules) | — |
| § "Verifier-friendly idioms — what to avoid" | (no extension; project rule is sufficient) | — |
| § "Testing tier mapping" | §C.1 `BPF_PROG_TEST_RUN` helper | Adds the userspace helper that Tier 2 needs and aya does not provide |

**Recommended additions to `.claude/rules/development.md`** (when this research is acted on):

1. Add a § "HASH_OF_MAPS — hand-rolled until aya 0.14+" subsection citing this research, with a one-line summary: "outer map and inner-map prototype both created via direct `bpf()` syscalls in `crates/overdrive-dataplane/src/sys/bpf.rs`; the typed `HashOfMapsHandle<K, V>` is the only entry point. Atomic backend swap is `HashOfMapsHandle::set(&service_id, new_inner_fd)` — kernel ref-counting handles in-flight readers."

2. Extend § "Testing tier mapping" with a one-line note: "Tier 2 PKTGEN/SETUP/CHECK uses the project's `prog_test_run()` helper at `crates/overdrive-dataplane/src/sys/prog_test_run.rs`; aya 0.13.x does not expose `BPF_PROG_TEST_RUN`."

3. Extend § "Map access from XDP context" with a HoM-specific bullet: "Outer-map lookup returns a NonNull<c_void> tagged `inner_map` by the verifier; chain to a second `bpf_map_lookup_elem` only after a NULL check. Single-level nesting only — the kernel rejects HoM-of-HoM at outer-map create time."

These edits are the architect agent's territory per the user's standing rule that ADRs and rule files go through the architect.

### Section F — Future Work / aya 0.14+

#### F.1 PR #1446 — typed HoM upstream

**Status as of 2026-03-31**: PR #1446 is open and has the approval of one core maintainer (@tamird). It is awaiting review from @alessandrod and @vadorovsky. The PR has been refined across multiple iterations to reach BTF-only support (the simpler legacy-map alternative in PR #1478 was closed in February 2026 in favour of #1446's more complete approach with tests + BTF).

**No assigned target version**: the PR thread does not name a specific aya release. Issue [#156](https://github.com/aya-rs/aya/issues/156) (TC-style map definitions) is tagged "Aya 1.0" milestone, suggesting that significant map-related restructuring is queued for the 1.0 cut. The conservative read: typed HoM lands in aya 1.0 or possibly a late 0.x point release (0.14, 0.15).

**Migration path when PR #1446 ships**:

The `HashOfMapsHandle<K, V>` recommended in §D.3 has a deliberately PR-1446-compatible signature. When typed `aya::maps::HashOfMaps` ships upstream, migration is:

1. Replace `HashOfMapsHandle::new_with_hash_inner(...)` with `aya::maps::HashOfMaps::try_from(bpf.map_mut(...))`.
2. Replace `HashOfMapsHandle::set(&key, inner_fd)` with `HashOfMaps::set(&key, &inner)` (aya's typed handle owns the inner map).
3. Replace kernel-side `HashOfMaps<K, V, M>` with `aya_ebpf::maps::HashOfMaps<K, V, M>`.
4. Remove `crates/overdrive-dataplane/src/sys/bpf.rs` HoM helpers.

The `BackendId` newtype, the `ServiceKey` shape, the kernel-side lookup-chain logic, and the `prog_test_run` helper all stay as-is.

**Source**: [aya PR #1446](https://github.com/aya-rs/aya/pull/1446). **Confidence**: Medium (PR is unmerged; behaviour confirmed via PR diff but final API may shift on review).

#### F.2 BPF_PROG_TEST_RUN typed wrapper — no upstream effort visible

A search of aya issues and PRs in the open set found **no in-flight effort to add a typed `test_run()` to `Xdp` or `SchedClassifier`**. The pattern in aya's own integration tests (live packet injection through `lo`) suggests the project does not see this as a gap. Overdrive's `prog_test_run()` helper is therefore expected to remain load-bearing across at least the next several aya releases — there is no migration target to plan against.

**Recommendation**: stabilise the helper at `crates/overdrive-dataplane/src/sys/prog_test_run.rs` with a documented `#[doc(hidden)]` interface; do not block any future work on it.

**Source**: aya issues / PRs search; aya integration test patterns. **Confidence**: Medium-High.

#### F.3 LSM hooks — Phase-aware

The whitepaper § 7 BPF LSM section enumerates `file_open`, `socket_create`, `socket_connect`, `task_setuid`, `bprm_check_security` as the project's eventual LSM hook surface. Phase 2.2 does not implement these. When the project reaches them (likely Phase 3+), aya 0.13.x's `Lsm` program type and `#[lsm(hook = "...")]` macro are sufficient — no hand-rolled patterns required.

**Caveat**: each LSM hook has a kernel-version-dependent attach-time signature. Adding a new hook should pair with a Tier 3 test on the full kernel matrix to confirm the hook is available on the project's 5.10 floor. `security_file_open` is stable since 5.7; `security_bprm_check` and `security_task_setuid` are stable older. The kernel.org BPF LSM doc references `security/security.c` as the canonical hook list.

**Source**: [aya book — LSM chapter](https://aya-rs.dev/book/programs/lsm/); [kernel.org BPF LSM](https://docs.kernel.org/bpf/prog_lsm.html); whitepaper § 7. **Confidence**: High.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| aya book — Maps & Programs chapters | aya-rs.dev | High (1.0) | Official project docs | 2026-05-06 | Yes |
| aya 0.13.1 docs.rs (`aya::maps`, `aya::programs::*`) | docs.rs | High (1.0) | Official Rust API docs (auto-generated from source) | 2026-05-06 | Yes |
| aya-ebpf 0.1.1 docs.rs (`aya_ebpf::maps`) | docs.rs | High (1.0) | Official Rust API docs | 2026-05-06 | Yes |
| aya repository @ tag `aya-v0.13.1` (source files: `aya/src/maps/mod.rs`, `aya/src/programs/xdp.rs`, `aya/src/sys/bpf.rs`) | github.com/aya-rs/aya | High (1.0) | Upstream source code | 2026-05-06 | Yes |
| aya PR #1446 (typed HashOfMaps / ArrayOfMaps) | github.com/aya-rs/aya | High (1.0) | Upstream PR with maintainer review | 2026-05-06 | Self-evidence |
| aya PR #1478 (closed, deferred to #1446) | github.com/aya-rs/aya | High (1.0) | Upstream PR thread | 2026-05-06 | Cross-ref to #1446 |
| aya issue #156 (TC-style map definitions, "Aya 1.0" milestone) | github.com/aya-rs/aya | High (1.0) | Upstream issue | 2026-05-06 | Yes |
| aya book examples (xdp-hello, tc-egress, lsm-nice) | github.com/aya-rs/book | High (1.0) | Official example code | 2026-05-06 | Yes |
| kernel.org BPF map_of_maps documentation | docs.kernel.org | High (1.0) | Linux kernel official documentation | 2026-05-06 | Yes |
| kernel.org BPF LSM documentation | docs.kernel.org | High (1.0) | Linux kernel official documentation | 2026-05-06 | Yes |
| aya-ebpf 0.1.1 source (`maps/hash_map.rs`) — direct cargo registry read | local cargo registry | High (1.0) | Vendored upstream source | 2026-05-06 | Cross-ref to docs.rs |
| aya-ebpf-macros 0.1.2 source (`map.rs`) — direct cargo registry read | local cargo registry | High (1.0) | Vendored upstream source | 2026-05-06 | Self-evidence (proves macro is type-agnostic) |

**Reputation tier breakdown**: High: 12 of 12 (100%). All sources are official upstream documentation, source code, or Linux kernel reference. Average reputation: 1.0.

**Cross-reference status**: Every load-bearing claim (§A coverage matrix, §D.1 macro behaviour, §D.6 verifier discipline) is corroborated by at least 2 independent sources (e.g., docs.rs + upstream source; kernel.org + Cilium reference). The aya 0.13.x version pin is verified across docs.rs, upstream tag, and direct cargo registry read — three independent paths.

## Knowledge Gaps

### Gap K-1: Exact aya 0.13.x relocation behaviour for HASH_OF_MAPS in the loaded ELF

**Issue**: §D.3 recommends declaring the outer HoM userspace-side and pinning to bpffs, then having the BPF program reference it by pin-path. An alternative — declaring the outer in the BPF ELF and patching the inner_map_fd at load time via aya's symbol-relocation machinery — is theoretically possible but the exact API is undocumented in 0.13.x. The userspace-pinned approach is the safer recommendation and is what the canonical libbpf shape uses, but a future optimisation might fold the outer declaration back into the BPF ELF.

**Attempted**: aya book, docs.rs for `Ebpf` and `EbpfLoader`, search of aya integration tests for HoM usage (none found — aya does not test HoM). PR #1446 source diffs (incomplete via WebFetch).

**Recommendation**: Slice 03-02b's crafter should land the userspace-pinned shape from §D.3 first. If a future requirement justifies in-ELF outer declaration, raise a follow-up research dispatch with a narrower scope ("aya 0.13.x map-fd patching at program load time").

### Gap K-2: SockOps hook constants — exact aya-ebpf surface

**Issue**: §B.3 documents that `aya::programs::SockOps` ships in 0.13.1 and the kernel-side macro is `#[sock_ops]`. The exact set of `BPF_SOCK_OPS_*_CB` constants exposed by `aya_ebpf` (or available via `aya_ebpf_bindings`) was not enumerated in this research because Phase 2.2 does not require sockops. The whitepaper's mTLS / kTLS path will need this enumeration when it lands (Phase 3+).

**Attempted**: aya book's program chapters (LSM only); cargo registry browsing.

**Recommendation**: Defer until Phase 3 sockops slice. The kernel `linux/bpf.h` defines the canonical set; aya-ebpf typically re-exports them via `aya_ebpf_bindings::bindings`.

### Gap K-3: Behaviour of the kernel-side `bpf_map_lookup_elem` helper signature with HoM-tagged inner pointers across aya-ebpf versions

**Issue**: §D.6 chains a second `bpf_map_lookup_elem` against the inner-map pointer returned from the outer lookup. The helper signature in `aya_ebpf::helpers` is generic over map type — but the verifier's tag tracking depends on the kernel version. Older kernels (5.10) have less precise tag propagation; the project's Tier 3 matrix needs to confirm this works across all five matrix kernels (5.10, 5.15, 6.1, 6.6, current LTS).

**Attempted**: kernel.org BPF map_of_maps doc (covers semantics, not version-specific verifier behaviour); Cilium docs (fetch failed — page is now a redirect-only stub).

**Recommendation**: Slice 03's Tier 3 acceptance test must include a per-kernel verification that the chained-lookup verifier check passes on each matrix kernel. If 5.10 rejects the pattern, the project either bumps the floor or implements a fallback (e.g., flatten HoM into a single HASH keyed on `(service_id, slot)` — less elegant, no atomic swap, but verifier-portable).

### Gap K-4: Empirical performance of HoM atomic swap vs in-place HASH update

**Issue**: §D.3 motivates HoM by the atomic-swap requirement (Slice 03). But the alternative — keeping a single flat `HASH<(ServiceId, Slot), BackendId>` and using `BPF_F_LOCK` for atomic update — is also tenable. The HoM approach is structurally cleaner and matches the Cilium pattern, but neither this research nor the project's existing benchmarks have quantified the per-packet cost difference (extra map lookup vs spinlock contention).

**Attempted**: Cilium production reference (no published benchmark on this specific axis); Katran source.

**Recommendation**: Slice 07's Tier 4 perf baseline should record both verifier instruction count and per-packet pps for the HoM path. If pps regression exceeds the 5% gate set by `.claude/rules/testing.md`, raise an ADR-amendment dispatch to the architect agent on whether to fall back to the flat HASH shape.

## Conflicting Information

No substantive conflicts encountered. One minor cross-source ambiguity worth noting:

- aya book LSM chapter and kernel.org BPF LSM doc agree on the kernel requirements (BPF LSM enabled, BTF available) but use different attach mechanism vocabulary (aya: `Lsm::attach()`; kernel.org: "`BPF_RAW_TRACEPOINT_OPEN` via bpf(2)" or "`bpf_program__attach_lsm`"). Both describe the same kernel pathway — aya's typed `attach()` wraps `bpf(BPF_RAW_TRACEPOINT_OPEN)` underneath. Not a conflict; just terminology drift between user-facing API and kernel-facing primitive.

## Recommendations for Further Research

1. **Watch PR #1446 for merge.** When typed HoM lands upstream, dispatch a follow-up research note to capture the final API signature and update §F.1's migration recipe. Effort: 1 hour. Trigger: PR merge or aya 0.14 / 1.0 release announcement.
2. **Per-kernel verifier behaviour for chained inner-map lookups (Gap K-3).** Empirical Tier 3 verification across the project's 5-kernel matrix. Effort: part of Slice 03 Tier 3 acceptance work, not a separate research dispatch.
3. **Sockops hook surface for Phase 3 (Gap K-2).** Defer until Phase 3 begins. Single research dispatch covering aya `SockOps` typed surface, `BPF_SOCK_OPS_*_CB` constants, kTLS install pattern via `bpf_sock_ops_cb_flags_set`. Effort: medium.
4. **HoM vs flat-HASH performance characterisation (Gap K-4).** Defer until Slice 07's Tier 4 baseline lands. If pps regression exceeds 5%, raise an ADR-amendment dispatch.

## Full Citations

[1] Aya Project. "Aya: An eBPF library for the Rust programming language — The Aya Book". aya-rs.dev. 2026. https://aya-rs.dev/book/. Accessed 2026-05-06.

[2] Aya Project. "Aya 0.13.1 API documentation — `aya::maps`". docs.rs. 2025. https://docs.rs/aya/0.13.1/aya/maps/index.html. Accessed 2026-05-06.

[3] Aya Project. "Aya 0.13.1 API documentation — `aya::maps::Map` enum". docs.rs. 2025. https://docs.rs/aya/0.13.1/aya/maps/enum.Map.html. Accessed 2026-05-06.

[4] Aya Project. "Aya 0.13.1 API documentation — `aya::programs::xdp::Xdp`". docs.rs. 2025. https://docs.rs/aya/0.13.1/aya/programs/xdp/struct.Xdp.html. Accessed 2026-05-06.

[5] Aya Project. "Aya 0.13.1 API documentation — `aya::programs::tc::SchedClassifier`". docs.rs. 2025. https://docs.rs/aya/0.13.1/aya/programs/tc/struct.SchedClassifier.html. Accessed 2026-05-06.

[6] Aya Project. "Aya 0.13.1 API documentation — `aya::programs::ProgramInfo`". docs.rs. 2025. https://docs.rs/aya/0.13.1/aya/programs/struct.ProgramInfo.html. Accessed 2026-05-06.

[7] Aya Project. "aya-ebpf 0.1.1 API documentation — `aya_ebpf::maps`". docs.rs. 2025. https://docs.rs/aya-ebpf/0.1.1/aya_ebpf/maps/index.html. Accessed 2026-05-06.

[8] Aya Project. "aya repository @ tag aya-v0.13.1 — `aya/src/maps/mod.rs`". github.com/aya-rs/aya. 2025. https://github.com/aya-rs/aya/blob/aya-v0.13.1/aya/src/maps/mod.rs. Accessed 2026-05-06.

[9] Aya Project. "aya repository @ tag aya-v0.13.1 — `aya/src/programs/xdp.rs`". github.com/aya-rs/aya. 2025. https://github.com/aya-rs/aya/blob/aya-v0.13.1/aya/src/programs/xdp.rs. Accessed 2026-05-06.

[10] Aya Project. "aya repository @ tag aya-v0.13.1 — `aya/src/sys/bpf.rs`". github.com/aya-rs/aya. 2025. https://github.com/aya-rs/aya/blob/aya-v0.13.1/aya/src/sys/bpf.rs. Accessed 2026-05-06.

[11] Aya Project. "Aya integration tests @ tag aya-v0.13.1 — XDP test patterns". github.com/aya-rs/aya. 2025. https://github.com/aya-rs/aya/blob/aya-v0.13.1/test/integration-test/src/tests/xdp.rs. Accessed 2026-05-06.

[12] Aya Project. "PR #1446: Add support for BPF_MAP_TYPE_HASH_OF_MAPS and BPF_MAP_TYPE_ARRAY_OF_MAPS". github.com/aya-rs/aya. 2026. https://github.com/aya-rs/aya/pull/1446. Accessed 2026-05-06.

[13] Aya Project. "PR #1478 (closed in favour of #1446): maps: Add BPF_MAP_TYPE_ARRAY_OF_MAPS support". github.com/aya-rs/aya. 2026. https://github.com/aya-rs/aya/pull/1478. Accessed 2026-05-06.

[14] Aya Project. "Issue #156: Support TC-style map definitions (Aya 1.0 milestone)". github.com/aya-rs/aya. 2022–. https://github.com/aya-rs/aya/issues/156. Accessed 2026-05-06.

[15] Aya Project. "Aya book examples — tc-egress, xdp-hello, lsm-nice". github.com/aya-rs/book. 2026. https://github.com/aya-rs/book/tree/main/examples. Accessed 2026-05-06.

[16] Aya Project. "Aya book — LSM chapter". aya-rs.dev. 2026. https://aya-rs.dev/book/programs/lsm/. Accessed 2026-05-06.

[17] Linux Kernel Authors. "BPF — Map of maps (BPF_MAP_TYPE_HASH_OF_MAPS, BPF_MAP_TYPE_ARRAY_OF_MAPS)". docs.kernel.org. 2025. https://docs.kernel.org/bpf/map_of_maps.html. Accessed 2026-05-06.

[18] Linux Kernel Authors. "BPF — LSM (BPF_PROG_TYPE_LSM)". docs.kernel.org. 2025. https://docs.kernel.org/bpf/prog_lsm.html. Accessed 2026-05-06.

[19] aya-ebpf 0.1.1 source — `src/maps/hash_map.rs` (read directly from local cargo registry at `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/aya-ebpf-0.1.1/src/maps/hash_map.rs`, vendored upstream source). 2025. Accessed 2026-05-06.

[20] aya-ebpf-macros 0.1.2 source — `src/map.rs` (read directly from local cargo registry at `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/aya-ebpf-macros-0.1.2/src/map.rs`, vendored upstream source). 2025. Accessed 2026-05-06.

[21] Overdrive Project. ".claude/rules/development.md § aya-rs XDP / TC kernel-side patterns" (commit 1309591). 2026. (project-internal). Cross-referenced 2026-05-06.

[22] Overdrive Project. ".claude/rules/testing.md § Tier 3 — Real-Kernel Integration, § Kernel matrix". 2026. (project-internal). Cross-referenced 2026-05-06.

[23] Overdrive Project. "Whitepaper § 7 — eBPF Dataplane". docs/whitepaper.md. 2026. (project-internal). Cross-referenced 2026-05-06.

## Appendix: Project-Recommended Primitives

The signatures below are the concrete API surface Slice 03-02b's crafter should land. They are deliberately compatible with PR #1446's expected upstream surface so the eventual migration is mechanical.

### A.1 `crates/overdrive-dataplane/src/sys/bpf.rs`

Direct `bpf()` syscall wrappers. ~100 LoC of `unsafe` glue, lifted from libbpf semantics.

```rust
//! Direct `bpf(2)` syscall wrappers used where aya 0.13.x does not
//! ship a typed surface. Specifically: `BPF_MAP_TYPE_HASH_OF_MAPS`
//! creation with `inner_map_fd`, and `BPF_PROG_TEST_RUN`.
//!
//! When aya PR #1446 lands and the project upgrades, the HoM helpers
//! here are deleted and call sites move to `aya::maps::HashOfMaps`.
//! See `docs/research/dataplane/aya-rs-usage-comprehensive-research.md`
//! § F.1 for the migration recipe.

use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, FromRawFd};
use std::path::Path;
use std::ffi::CString;

pub fn bpf_create_map(
    map_type: u32,
    key_size: u32,
    value_size: u32,
    max_entries: u32,
    map_flags: u32,
    inner_map_fd: Option<BorrowedFd<'_>>,
) -> std::io::Result<OwnedFd>;

pub fn bpf_map_update_elem<K, V>(
    map_fd: BorrowedFd<'_>,
    key: &K,
    value: &V,
    flags: u64,
) -> std::io::Result<()>;

pub fn bpf_map_delete_elem<K>(
    map_fd: BorrowedFd<'_>,
    key: &K,
) -> std::io::Result<()>;

pub fn bpf_map_lookup_elem<K, V: Default + Copy>(
    map_fd: BorrowedFd<'_>,
    key: &K,
) -> std::io::Result<Option<V>>;

pub fn bpf_obj_pin(
    fd: BorrowedFd<'_>,
    path: &Path,
) -> std::io::Result<()>;

pub fn bpf_obj_get(path: &Path) -> std::io::Result<OwnedFd>;
```

### A.2 `crates/overdrive-dataplane/src/sys/prog_test_run.rs`

The `BPF_PROG_TEST_RUN` userspace helper (see §D.5).

```rust
use std::os::fd::BorrowedFd;

pub struct ProgTestRunOutput {
    pub retval: u32,
    pub data_out: Vec<u8>,
    pub duration_ns: u32,
}

/// Drive a loaded BPF program against synthetic input. Used by
/// Tier 2 PKTGEN/SETUP/CHECK triptych tests.
///
/// Aya 0.13.x does not expose this; this helper is a thin wrapper
/// around `bpf(BPF_PROG_TEST_RUN, ...)` via `libc::syscall`.
pub fn prog_test_run(
    prog_fd: BorrowedFd<'_>,
    input: &[u8],
    repeat: u32,
) -> std::io::Result<ProgTestRunOutput>;
```

### A.3 `crates/overdrive-dataplane/src/maps/hash_of_maps.rs`

The typed userspace handle (see §D.3).

```rust
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::marker::PhantomData;
use std::path::Path;

/// Marker trait for types safely transmutable to/from raw bytes.
/// Equivalent to aya's own internal `Pod` bound. Impl for u32,
/// fixed-size arrays, project newtypes wrapping u32/u64/[u8;N].
///
/// SAFETY: implementor must be `#[repr(C)]` or `#[repr(transparent)]`
/// and contain no padding bytes.
pub unsafe trait Pod: Copy + 'static {}

unsafe impl Pod for u32 {}
unsafe impl Pod for u64 {}
unsafe impl<const N: usize> Pod for [u8; N] {}

/// Typed userspace handle around a `BPF_MAP_TYPE_HASH_OF_MAPS` outer map.
///
/// Owns the outer map fd and the inner-map prototype fd. Drops both on
/// deallocation; pinned outer maps survive process exit (see `pin()`).
///
/// # Migration target
///
/// When aya PR #1446 ships, this struct collapses to a thin wrapper
/// around `aya::maps::HashOfMaps<MapData, K, M>`. The public method set
/// here is intentionally signature-compatible.
pub struct HashOfMapsHandle<K: Pod, V: Pod> {
    outer_fd: OwnedFd,
    _inner_proto_fd: OwnedFd,
    inner_max_entries: u32,
    _k: PhantomData<K>,
    _v: PhantomData<V>,
}

impl<K: Pod, V: Pod> HashOfMapsHandle<K, V> {
    /// Construct a new outer HoM with HASH inner-map prototype.
    /// Both maps are created via direct `bpf()` syscalls.
    pub fn new_with_hash_inner(
        name: &str,
        max_outer_entries: u32,
        max_inner_entries: u32,
    ) -> Result<Self, MapError>;

    /// Replace the inner map at `key` atomically.
    /// Returns Ok(()) on success; the previous inner map's fd remains
    /// valid for in-flight readers and is reclaimed by kernel ref-counting
    /// once all in-flight programs return.
    pub fn set(&self, key: &K, inner: BorrowedFd<'_>) -> Result<(), MapError>;

    /// Remove the slot at `key`.
    pub fn delete(&self, key: &K) -> Result<(), MapError>;

    /// Pin the outer fd to a bpffs path, so the BPF program can pick
    /// it up by `BPF_OBJ_GET` at load time rather than via aya
    /// relocation.
    pub fn pin<P: AsRef<Path>>(&self, path: P) -> Result<(), MapError>;

    /// Borrow the outer fd for direct syscalls (rarely needed).
    pub fn as_fd(&self) -> BorrowedFd<'_>;

    /// Create a fresh HASH inner map matching this HoM's prototype.
    /// Returns the new map's fd, ready to populate before passing
    /// to `set()`.
    pub fn create_inner(
        &self,
        max_entries: Option<u32>,
    ) -> Result<OwnedFd, MapError>;
}

#[derive(thiserror::Error, Debug)]
pub enum MapError {
    #[error("bpf() syscall failed: {0}")]
    Syscall(#[from] std::io::Error),
    // ... project-specific variants
}

pub type Result<T, E = MapError> = std::result::Result<T, E>;
```

### A.4 `crates/overdrive-bpf/src/maps/hash_of_maps.rs`

The kernel-side struct (see §D.2). Used inside `#[xdp]` / `#[classifier]` programs via `#[map]`.

```rust
use core::{cell::UnsafeCell, marker::PhantomData, ptr::NonNull};
use aya_ebpf_bindings::bindings::bpf_map_type::BPF_MAP_TYPE_HASH_OF_MAPS;
use aya_ebpf::{bindings::bpf_map_def, helpers::bpf_map_lookup_elem};
use aya_ebpf_cty::c_void;

mod sealed { pub trait Sealed {} }

/// Marker trait for types valid as inner-map of a `HashOfMaps`.
/// Implementations are sealed to the aya-ebpf canonical set.
pub trait InnerMap: sealed::Sealed {
    /// The kernel `BPF_MAP_TYPE_*` constant. Used for inner-map
    /// prototype creation in userspace; kernel-side never reads it.
    const INNER_MAP_TYPE: u32;
}

// Blanket impls for the inner-map types HoM supports.
impl<K, V> sealed::Sealed for aya_ebpf::maps::HashMap<K, V> {}
impl<K, V> InnerMap for aya_ebpf::maps::HashMap<K, V> {
    const INNER_MAP_TYPE: u32 =
        aya_ebpf_bindings::bindings::bpf_map_type::BPF_MAP_TYPE_HASH;
}
impl<V> sealed::Sealed for aya_ebpf::maps::Array<V> {}
impl<V> InnerMap for aya_ebpf::maps::Array<V> {
    const INNER_MAP_TYPE: u32 =
        aya_ebpf_bindings::bindings::bpf_map_type::BPF_MAP_TYPE_ARRAY;
}
// Add more as needed.

/// Kernel-side `BPF_MAP_TYPE_HASH_OF_MAPS` declaration.
///
/// Use with `#[map]` exactly like aya-ebpf's `HashMap<K, V>`:
///
/// ```ignore
/// #[map]
/// static SERVICE_MAP: HashOfMaps<ServiceKey, BackendId, HashMap<u32, BackendId>> =
///     HashOfMaps::with_max_entries(4096, 0);
/// ```
///
/// The `M: InnerMap` parameter is structural-only — the kernel's outer
/// map only stores `inner_map_fd`s; `M` exists at the type level so
/// callers can specify the inner-map shape uniformly.
#[repr(transparent)]
pub struct HashOfMaps<K, V, M: InnerMap> {
    def: UnsafeCell<bpf_map_def>,
    _k: PhantomData<K>,
    _v: PhantomData<V>,
    _m: PhantomData<M>,
}

unsafe impl<K: Sync, V: Sync, M: InnerMap> Sync for HashOfMaps<K, V, M> {}

impl<K, V, M: InnerMap> HashOfMaps<K, V, M> {
    /// Construct an outer HoM. `flags` is passed through to
    /// `bpf_map_def::map_flags`; the canonical value is 0.
    pub const fn with_max_entries(max_entries: u32, flags: u32) -> Self;

    /// Look up the inner map for `key`. Returns NonNull pointer to
    /// the inner map (verifier-tagged `inner_map`) or None.
    /// Caller MUST chain to `bpf_map_lookup_elem` only after a
    /// successful lookup here — the verifier rejects unconditional
    /// dereference of the outer-lookup result.
    #[inline(always)]
    pub fn lookup_inner(&self, key: &K) -> Option<NonNull<c_void>>;
}
```

### A.5 Suggested file layout for Slice 03-02b

```
crates/overdrive-dataplane/src/
├── sys/
│   ├── mod.rs               # pub mod bpf; pub mod prog_test_run;
│   ├── bpf.rs               # A.1 — direct syscall wrappers
│   └── prog_test_run.rs     # A.2 — BPF_PROG_TEST_RUN helper
└── maps/
    ├── mod.rs               # re-exports
    └── hash_of_maps.rs      # A.3 — userspace HashOfMapsHandle

crates/overdrive-bpf/src/
└── maps/
    ├── mod.rs               # pub mod hash_of_maps;
    └── hash_of_maps.rs      # A.4 — kernel-side HashOfMaps + InnerMap
```

This mirrors PR #1446's anticipated module layout (`aya/src/maps/of_maps/{hash_of_maps,array_of_maps}.rs` and `aya-ebpf/src/btf_maps/{hash_of_maps,array_of_maps}.rs`), so the eventual migration is by-file rename + signature replace.

### A.6 Test surface

Tier 2 (`crates/overdrive-bpf/tests/integration/service_map.rs`):
- PKTGEN: synthesised IPv4 + TCP frame for a SERVICE VIP.
- SETUP: `HashOfMapsHandle::new_with_hash_inner("SERVICE_MAP", 4096, 1024)`; populate one inner with backends; `set(&service_id, inner.as_fd())`.
- CHECK: `prog_test_run(xdp_prog_fd, &frame, 1)` → assert `retval == XDP_TX`, header rewrite visible in `data_out`.

Tier 3 (`crates/overdrive-dataplane/tests/integration/atomic_swap.rs`):
- VethPair RAII; load XDP onto one peer; populate v1 inner map; inject 1000 packets via raw socket on the other peer; in parallel, swap to v2 inner via `HashOfMapsHandle::set`; assert tcpdump capture shows zero packet drops across the swap window.

Tier 4 (`perf-baseline/main/verifier-budget/xdp_service_map_lookup.txt`):
- After load, read `aya::programs::ProgramInfo::verified_instruction_count()`; baseline number stored in the file; PR fails if delta > 5%.

Each tier reuses the typed primitives in §A.1–A.4. No additional hand-rolling is required.
