# Research: Attaching Two Distinct XDP Programs to a Single Interface (aya 0.13.x) — Fixing Overdrive's Single-Node Loopback `EBUSY` Collision

**Date**: 2026-06-02 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (avg source reputation ≈ 0.99; 13/14 external sources high-tier; every major claim cross-referenced ≥ 2 ways) | **Sources**: 18 (14 external authoritative + repo primary source)

## Executive Summary

Overdrive's single-node `EbpfDataplane` aborts at boot with `EBUSY` because
`DataplaneConfig::loopback()` points both `client_iface` and `backend_iface`
at `lo`, and the kernel permits **exactly one program on a netdev's XDP
hook**. `bpf_link` (`BPF_LINK_TYPE_XDP`, ≥5.9) does not change this — it
improves attachment *lifecycle*, not the single-owner limit; and the attach
flags (`REPLACE`, `UPDATE_IF_NOEXIST`, `SKB_MODE`/`DRV_MODE`/`HW_MODE`)
manage the single slot, they do not add a second program. The collision is
real and intrinsic to attaching two distinct XDP programs to one interface.

There are five ways to make two XDP stages coexist on one interface: **(A)**
the libxdp `xdp_dispatcher` (freplace/`BPF_PROG_TYPE_EXT` chaining — the
canonical answer, kernel floor 5.10 for full incremental attach, **but aya
0.13.x ships only the `Extension` primitive, no dispatcher**, so it means a
multi-week hand-roll of the libxdp dispatcher ABI or a C-FFI dependency);
**(B)** merge the two programs into one staged `#[xdp]` entry (lowest
new-mechanism cost — the two programs already share the sanity prologue and
both early-return `XDP_PASS`, so it is mechanical; cost is a bounded,
gate-measured verifier re-baseline); **(C)** tail calls via `PROG_ARRAY`
(aya-native but strictly dominated by B for two fixed stages — more parts,
halved stack budget); **(D)** split hooks across layers (XDP ingress + TC/
`tcx` — legitimate Cilium-style, but for Overdrive it reverses the landed
ADR-0045 and re-ports reverse-NAT to the TC surface); **(E)** stop attaching
to `lo` and provision a dedicated veth pair (no BPF change; restores the
two-distinct-iface design the production code already assumes).

The recommendation: **fix the target, not just the symptom.** `lo` is the
wrong attach point on two counts — the single-XDP-slot collision *and* a
deeper correctness problem (generic/SKB-mode XDP, the only mode `lo`
supports, can be bypassed for cloned skbs on the TCP retransmit path, so a
loopback dataplane may silently miss traffic even after the collision is
resolved). **Option E (dedicated veth pair)** is the primary recommendation:
it removes `EBUSY`, restores correct XDP semantics, matches the project's
own ADR-0043 Tier-3 veth topology, needs zero kernel-side change, and mirrors
how Cilium/Katran deploy. **Option B (merge programs)** is the recommended
fallback and composes well with E to unify the single-node and two-NIC
attach paths. C, D, and A are not recommended for this fix (dominated,
disruptive, and over-engineered respectively). The one gating unknown for E
is single-node traffic-steering through the veth client side (Gap G-4) — a
focused design task, not a research blocker.

## Research Methodology

**Search Strategy**: Primary sources — kernel.org / docs.kernel.org (XDP, `bpf_link`, `XDP_FLAGS_*`), the aya book + `github.com/aya-rs/aya` (source/issues/PRs), `github.com/xdp-project/xdp-tools` (libxdp `xdp_dispatcher`), lwn.net (kernel journalism), Cilium docs. Cross-referenced against the existing repo research doc `aya-rs-usage-comprehensive-research.md` and the live code in `crates/overdrive-dataplane/src/lib.rs`.

**Source Selection**: Official kernel/aya/libxdp primary sources preferred over blog posts. Every technical claim carries kernel version + aya version + access date.

**Quality Standards**: 2–3 sources/claim; 1 authoritative (kernel docs / aya source) minimum where API surface has a single SSOT.

## The Concrete Problem (repo-grounded)

`EbpfDataplane::new_with_pin_dir` (`crates/overdrive-dataplane/src/lib.rs`)
attaches **two distinct** XDP programs:

- `xdp_service_map_lookup` → `client_iface` ingress (forward / service-map path), ~L489.
- `xdp_reverse_nat_lookup` → `backend_iface` ingress (reverse-NAT path), ~L534.

Each call uses `XdpFlags::DRV_MODE` with an `EOPNOTSUPP/ENOTSUP → SKB_MODE`
fallback (the documented attach-mode rule). The two-iface design is correct
for the two-NIC / two-veth production topology (ADR-0043's three-iface test
topology; ADR-0045 backend-facing veth ingress).

The single-node default collapses both ifaces onto loopback. From
`crates/overdrive-control-plane/src/dataplane_config.rs`:

```rust
pub fn loopback() -> Self {
    Self { client_iface: "lo".to_owned(), backend_iface: "lo".to_owned() }
}
```

So the two `attach` calls target the **same** netdev XDP hook (`lo`). The
kernel permits exactly one XDP program on a netdev's XDP hook absent a
multiprog dispatcher (Section 1), so the second `attach` returns `EBUSY`
and `overdrive serve` aborts at boot. This research enumerates the real
options to make two XDP stages coexist on one interface and ranks them for
Overdrive's Phase-1 single-node-in-scope constraint.

## Section 1 — The Kernel Model: one XDP program per interface, `EBUSY`, and `bpf_link`

### Finding 1.1: A netdev's XDP hook holds exactly one program

**Evidence**: "The API enforces a one-program-per-interface constraint via
the standard XDP attachment mechanism. Only a single XDP program can be
actively attached to any given network interface at one time through this
approach." The XDP hook is a single `bpf_prog` pointer on the netdev
(`net_device->xdp_prog` historically), not a list — attaching a second
independent program is structurally impossible at the hook level, which is
exactly why libxdp exists (Section 2).
**Source**: [eBPF Docs — `bpf_xdp_attach` (libbpf)](https://docs.ebpf.io/ebpf-library/libbpf/userspace/bpf_xdp_attach/) — Accessed 2026-06-02
**Confidence**: High
**Verification**: [eBPF Docs — `BPF_PROG_TYPE_XDP`](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_XDP/), [libxdp README.org](https://github.com/xdp-project/xdp-tools/blob/main/lib/libxdp/README.org) (the dispatcher's entire reason for existing is this single-slot constraint).
**Analysis**: This is the root cause of the Overdrive `EBUSY`. Two distinct
programs cannot own `lo`'s XDP hook simultaneously.

### Finding 1.2: `EBUSY` is the "already attached" signal; `UPDATE_IF_NOEXIST` / `REPLACE` govern it

**Evidence**: "If you attempt to attach a program without the REPLACE flag
and another program is already attached, the system returns an EBUSY error."
`XDP_FLAGS_UPDATE_IF_NOEXIST` "only attaches if no program is already
attached to the interface" — it is the *guard against* clobbering, not a
multi-program enabler. `XDP_FLAGS_REPLACE` swaps the existing program for a
new one (single-slot semantics preserved — the old program is detached).
**Source**: [eBPF Docs — `bpf_xdp_attach`](https://docs.ebpf.io/ebpf-library/libbpf/userspace/bpf_xdp_attach/) — Accessed 2026-06-02
**Confidence**: High
**Verification**: WebSearch consensus (isovalent/ebpf-docs, cilium/ebpf discussion #483) confirms: second attach without flags → `EBUSY`.
**Analysis**: Overdrive's second `attach(lo, DRV_MODE)` hits this exactly.
`REPLACE` does NOT help — it would detach the forward program. Neither flag
makes two programs coexist; they manage the single slot.

### Finding 1.3: The XDP attach *flags* are mode/guard bits, not multi-program bits

**Evidence**: The flag set is `XDP_FLAGS_SKB_MODE` (generic, full netstack
traversal — works on any driver incl. `lo`), `XDP_FLAGS_DRV_MODE` (native,
driver hook), `XDP_FLAGS_HW_MODE` (hardware offload), `XDP_FLAGS_REPLACE`
(atomic swap of the single program), `XDP_FLAGS_UPDATE_IF_NOEXIST` (refuse
if a program is present). None of them attach a *second* program.
**Source**: [eBPF Docs — `BPF_PROG_TYPE_XDP`](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_XDP/) — Accessed 2026-06-02
**Confidence**: High
**Verification**: aya `XdpFlags` surface (`DEFAULT`, `DRV_MODE`, `SKB_MODE`,
`HW_MODE`, `REPLACE`, `UPDATE_IF_NOEXIST`) per the repo's own
`aya-rs-usage-comprehensive-research.md` §B.1 — 1:1 mapping to the kernel flags.
**Analysis**: Note: `lo` has no native XDP driver, so on loopback both
Overdrive attaches resolve to `SKB_MODE` (generic) regardless — but the
single-slot constraint applies identically in generic mode. The first
attach succeeds; the second `EBUSY`s.

### Finding 1.4: `bpf_link`-based XDP (`BPF_LINK_TYPE_XDP`, kernel ≥ 5.9) does NOT add multi-program ownership

**Evidence**: "The modern and recommended way to attach XDP programs is to
use BPF links … by calling BPF_LINK_CREATE with the target_ifindex set …
and attach_type set to BPF_LINK_TYPE_XDP." Links improve *lifecycle*
management (the link FD owns the attachment; close → detach) and the
`UPDATE_IF_NOEXIST` flag "is only used with the netlink attach method; the
link attach method handles this behavior more generically." But the link
still targets the same single XDP hook — it does **not** permit two
independent link owners on one netdev's XDP hook.
**Source**: [WebSearch consensus — isovalent/ebpf-docs `bpf_xdp_attach.md`, cilium/ebpf discussion #483](https://github.com/cilium/ebpf/discussions/483) — Accessed 2026-06-02
**Confidence**: Medium-High (cross-source consensus; no single kernel-doc page quoted verbatim — see Knowledge Gap G-1)
**Verification**: libxdp README implies the same — even with `bpf_link`, multiprog still requires the dispatcher (the link points at the *dispatcher* program, not at N user programs).
**Analysis**: **Confirms the prompt's hypothesis**: `bpf_link` changes the
lifecycle/ownership *model* but not the single-program-per-hook *limit*.
aya 0.13.x's `Xdp::attach` returns an `XdpLinkId` (link-based on ≥5.9), and
Overdrive already uses it — switching attach styles will not fix `EBUSY`.

## Section 2 — Option A: libxdp `xdp_dispatcher` multiprog

### Finding 2.1: The dispatcher is an XDP program that `freplace`s stub functions with the user programs

**Evidence**: "The dispatcher is simply an XDP program that will call each of
a number of stub functions in turn, and depending on their return code
either continue on to the next function or return immediately. These stub
functions are then replaced at load time with the user XDP programs, using
the freplace functionality." Programs sort by **run priority** (default 50;
lower runs first); the dispatcher continues to the next program only if the
prior returns a **chain-call action** (default `XDP_PASS`), otherwise it
returns that verdict immediately.
**Source**: [libxdp README.org](https://github.com/xdp-project/xdp-tools/blob/main/lib/libxdp/README.org) — Accessed 2026-06-02
**Confidence**: High
**Verification**: [eBPF Docs — libxdp Concept](https://docs.ebpf.io/ebpf-library/libxdp/libxdp/); [Toke Høiland-Jørgensen, "XDP multiprog", Linux Plumbers Conf](https://lpc.events/event/7/contributions/671/attachments/561/992/xdp-multiprog.pdf) (the dispatcher's origin talk).
**Analysis**: For Overdrive this maps cleanly: `xdp_service_map_lookup` at
priority N, `xdp_reverse_nat_lookup` at priority N+1, both with `XDP_PASS`
as a chain-call action so a "not my traffic" early-return falls through to
the next stage. Both already early-return `XDP_PASS` on traffic they don't
own — the chaining semantics fit.

### Finding 2.2: Kernel floor — 5.6 (multiprog if all-at-once), 5.10 (full incremental via freplace re-attach)

**Evidence**: "The full functionality … can only be attained with kernels
version 5.10 or newer, because this is the version that introduced support
for re-attaching an freplace program in a secondary attachment point.
However, the freplace functionality itself was introduced in kernel 5.7,
so for kernel versions 5.7 to 5.9, multiple programs can be attached as
long as they are all attached to the dispatcher immediately as they are
loaded." Multiprog at all requires 5.6; the underlying `BPF_PROG_TYPE_EXT`
("dynamic program extensions") landed via commit `be8704ff07d2` in 5.6.
**Architectures lacking BPF trampoline support stay single-program even on
5.10+.**
**Source**: [WebSearch consensus — libxdp README + netdev freplace multi-attach series](https://www.mail-archive.com/netdev@vger.kernel.org/msg356216.html) — Accessed 2026-06-02
**Confidence**: High
**Verification**: [libxdp README.org](https://github.com/xdp-project/xdp-tools/blob/main/lib/libxdp/README.org); [aya `Extension` docs.rs](https://docs.rs/aya/0.13.1/aya/programs/extension/struct.Extension.html) (min kernel 5.9 for the primitive).
**Analysis**: Overdrive's kernel floor is **5.10 LTS**
(`.claude/rules/testing.md` § Kernel matrix) — so the *full* freplace
incremental-attach path is available on the floor and above, on
trampoline-capable arches (x86-64, arm64 — both in the matrix). This is a
fit on the matrix, **but** see 2.4: there's no aya-native dispatcher.

### Finding 2.3: BTF + `-g` required; programs need type info

**Evidence**: Component programs "must include type information (using the
BPF Type Format, BTF)," requiring "a recent version of Clang/LLVM (version
10+), and … debug information when compiling (using the `-g` option)." The
freplace verifier uses the `(attach_prog_fd, attach_btf_id)` pair to
identify the function to replace; BTF type signatures of the stub and the
replacement must match.
**Source**: [libxdp README.org](https://github.com/xdp-project/xdp-tools/blob/main/lib/libxdp/README.org) — Accessed 2026-06-02
**Confidence**: High
**Verification**: [eBPF Docs — `BPF_PROG_TYPE_EXT`](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_EXT/); aya `Extension::load` "requires BTF compatibility verification."
**Analysis**: Overdrive's BPF ELF already emits BTF (required for the
project's existing CO-RE / map work — see `aya-rs-usage` §B.5). The BTF
prerequisite is already satisfied; no new toolchain burden.

### Finding 2.4: aya 0.13.x ships the `Extension` (freplace) primitive but **NOT** a pre-built `xdp_dispatcher`

**Evidence**: aya 0.13.1 exposes `aya::programs::Extension` (freplace,
min kernel 5.9) with `load(program, func_name)`, `attach()`,
`attach_to_program(program, func_name)`. It also exposes `MultiProgLink` /
`MultiProgram` traits — but those back the **kernel's generic multi-prog
API** (TCX, cgroup), **not** an XDP dispatcher. "The documentation provides
no evidence that Aya ships a pre-built xdp_dispatcher program. Extension
appears to be a raw primitive requiring users to write their own extension
code."
**Source**: [aya `Extension` docs.rs](https://docs.rs/aya/0.13.1/aya/programs/extension/struct.Extension.html), [aya `programs` index docs.rs](https://docs.rs/aya/0.13.1/aya/programs/index.html) — Accessed 2026-06-02
**Confidence**: High
**Verification**: aya does not link libxdp; there is no `aya::programs::xdp::XdpDispatcher` or equivalent in the 0.13.1 surface. The `MultiProgLink`/`MultiProgram` traits are for TCX/cgroup ordering, which is a *different* kernel mechanism from XDP freplace-dispatcher.
**Analysis**: This is the load-bearing constraint for Option A. To use the
libxdp dispatcher pattern under aya 0.13.x, Overdrive would have to
**hand-roll the dispatcher** — author a dispatcher XDP ELF with N stub
global functions, compute the dispatcher BTF, load the two user programs as
`Extension`s, and `attach_to_program` each against a stub. This is the same
class of "aya doesn't ship it, hand-roll over the primitive" work the
project already did for HASH_OF_MAPS (`aya-rs-usage` §D), but materially
**larger and riskier**: the dispatcher protocol (config-section layout,
priority sort, chain-call action bitmaps, the `.xdp_run_config` BTF
metadata) is a libxdp-defined ABI (`lib/libxdp/protocol.org`), not a single
syscall. Re-implementing it faithfully in Rust/aya is a multi-week effort
with no upstream aya support to lean on.

### Finding 2.5: Alternative within Option A — shell out to / link libxdp directly

**Evidence**: libxdp is a C library (`xdp-tools`) with a stable C API
(`xdp_program__*`, `xdp_multiprog__*`). A Rust binding (FFI / `bindgen`)
could load the two programs through libxdp's dispatcher rather than through
aya's loader.
**Source**: [libxdp(3) man page](https://manpages.debian.org/bookworm/libxdp-dev/libxdp.3.en.html) — Accessed 2026-06-02
**Confidence**: Medium-High
**Verification**: [Ubuntu libxdp(3) manpage](https://manpages.ubuntu.com/manpages/noble/man3/libxdp.3.html).
**Analysis**: This introduces a C-library runtime dependency (`libxdp.so` +
`libbpf.so`) into the production binary, plus an FFI surface that must stay
in sync with aya's view of the same maps/programs. It contradicts the
project's all-Rust/aya posture and the "hand-roll over raw `bpf()` rather
than add a C dep" pattern already chosen for HoM. High integration risk;
not recommended over the simpler Options B/D/E below.

**Option A verdict**: Technically sound and the "canonical" answer for
N-program XDP chaining on one iface, kernel-floor-compatible (5.10). But
aya 0.13.x has **no dispatcher** — either a multi-week hand-roll of the
libxdp dispatcher ABI, or a C-library FFI dependency. Disproportionate to a
two-program single-node default. **Over-engineered for this problem.**

## Section 3 — Option B: Merge the two programs into one XDP entry

### Finding 3.1: Single-program composition is the idiomatic XDP answer for "two stages on one iface"

**Evidence**: The kernel's own guidance treats sequential per-packet logic
as stages *within* one program (or chained via tail calls / dispatcher).
"This upper nesting limit of 33 calls is usually used to decouple parts of
program logic, for example, into stages" — staging is the norm; a single
XDP entry that runs stage 1 then stage 2 is the simplest realisation. XDP
exposes exactly one hook per netdev, so anything beyond "one program does
everything" needs a chaining mechanism (dispatcher/tail-call). When the
stages are few and statically known, in-lining them in one program is the
lowest-complexity choice.
**Source**: [eBPF Docs — Tail calls](https://docs.ebpf.io/linux/concepts/tail-calls/) — Accessed 2026-06-02
**Confidence**: High
**Verification**: [Cilium BPF Architecture](https://docs.cilium.io/en/latest/bpf/architecture/) (Cilium composes large per-packet logic in single programs + tail-calls between major stages, not via N independent attaches); aya book single-`#[xdp]`-program examples.
**Analysis**: Overdrive's two programs are *already* shaped as
early-returning stages: `xdp_service_map_lookup` returns `XDP_PASS` on a
SERVICE_MAP miss; `xdp_reverse_nat_lookup` returns `XDP_PASS` on a
REVERSE_NAT_MAP miss. A merged `xdp_dataplane(ctx)` that runs the
service-map stage, and on its `XDP_PASS` fall-through runs the reverse-NAT
stage, reproduces the two-iface behaviour on one hook with no new kernel
mechanism.

### Finding 3.2: The two programs already share structure — composition is mechanical

**Evidence** (repo, `crates/overdrive-bpf/src/programs/`): both programs run
the shared sanity prologue (`shared/sanity.rs`), parse the same
Ethernet/IPv4/L4 headers via the shared `ptr_at` helper, and produce the
same verdict vocabulary (`XDP_PASS` on "not my traffic", `XDP_DROP` on
sanity violation, `bpf_redirect`/`XDP_TX` on a hit). `xdp_reverse_nat_lookup`
adds `bpf_fib_lookup` + `bpf_redirect` (note: `bpf_redirect_neigh` is
TC-only and the verifier rejects it on XDP — ADR-0045, `xdp_reverse_nat.rs`
L33-36). The map sets are disjoint (SERVICE_MAP/BACKEND_MAP vs
REVERSE_NAT_MAP) and compose without key collision.
**Source**: repo `crates/overdrive-bpf/src/programs/{xdp_service_map.rs,xdp_reverse_nat.rs}`, `shared/sanity.rs` — Cross-referenced 2026-06-02
**Confidence**: High
**Verification**: ADR-0045 (`docs/product/architecture/adr-0045-bpf-redirect-neigh-datapath.md`); the shared sanity prologue is consumed by both.
**Analysis**: Because both stages already early-return `XDP_PASS` and share
the header-parse prologue, the merge factors as: run the shared prologue
once, then service-map stage, then (on fall-through) reverse-NAT stage. The
prologue runs once instead of twice — a small per-packet *win* on the merged
path.

### Finding 3.3: Verifier budget is the cost — and it is bounded/measurable

**Evidence**: Merging concatenates both programs' instruction footprints
into one verified program. The project gates verifier complexity at Tier 4
(`cargo verifier-regress`, reading `bpf_prog_info.verified_insns`), with a
documented ceiling of ≤ 50% of the 1M-privileged instruction limit per
program (`.claude/rules/testing.md` § Verifier complexity;
`aya-rs-usage` §C.3). Tail-call/BPF-to-BPF stack interaction (512B → 256B
when tail calls are in play) does **not** apply here — a merged single
program with no tail calls keeps the full 512B stack budget.
**Source**: [eBPF Docs — `bpf_tail_call`](https://docs.ebpf.io/linux/helper-function/bpf_tail_call/) (stack-limit interaction, here avoided) — Accessed 2026-06-02
**Confidence**: High
**Verification**: `.claude/rules/testing.md` § "Verifier complexity (`cargo verifier-regress`)"; the existing per-program baselines under `perf-baseline/main/`.
**Analysis**: Two ~moderate XDP programs concatenated are very unlikely to
approach 500K instructions — each is well under the budget today. The
verifier-budget gate will *measure* the merged program; if it regresses
past the ceiling the gate catches it pre-merge. Risk is bounded and
observable, not open-ended.

### Finding 3.4: The two-iface production path is *not* regressed by merging — if done carefully

**Evidence/Analysis** (interpretation, repo-grounded): A merged
`xdp_dataplane` program attached to a single iface still works on the
two-NIC production topology — you would attach the *same* merged program to
both `client_iface` and `backend_iface` (each iface's hook runs the merged
program; the service-map stage hits on client ingress, the reverse-NAT
stage hits on backend ingress, the other stage falls through as `XDP_PASS`).
**Alternatively**, keep the two distinct programs for the two-NIC path and
add the merged program only for the single-iface (`lo`) case — but that
doubles the kernel-side surface. The cleaner factoring is one merged
program attached to whatever iface(s) the config names (1 for single-node
`lo`, 2 for two-NIC). This *unifies* the attach path and removes the
special case.
**Confidence**: Medium (this is design interpretation; the verifier-budget
and per-iface-hit behaviour need a Tier 3 confirmation — see Gap G-2)
**Verification**: consistent with Finding 3.2 (disjoint maps, shared
prologue) and Finding 1.1 (one program per hook — a merged program is one
program, so it attaches cleanly on every iface).

**Option B verdict**: Lowest new-mechanism cost. No freplace, no dispatcher,
no tail-call map, no C dependency, no kernel-floor change (works on any
kernel that runs XDP at all). Cost is a one-time kernel-side refactor
(merge two `#[xdp]` bodies into one staged entry) plus a verifier-budget
re-baseline. Fits the project's all-Rust/aya posture and single-cut
greenfield migration discipline. **Strong candidate.**

## Section 4 — Option C: Tail calls (`BPF_MAP_TYPE_PROG_ARRAY`)

### Finding 4.1: Tail calls chain programs via a `PROG_ARRAY` + `bpf_tail_call`, up to 33 deep

**Evidence**: "To use tail calls, a `BPF_MAP_TYPE_PROG_ARRAY` map should be
added … filled with references to other programs, and the program can then
use the `bpf_tail_call` helper … to perform the actual tail call." The
nesting limit `MAX_TAIL_CALL_CNT` is 33. "Tail calls do not return to the
call site but instead run as if they were invoked by the kernel directly" —
control transfers, it does not return.
**Source**: [eBPF Docs — `BPF_MAP_TYPE_PROG_ARRAY`](https://docs.ebpf.io/linux/map-type/BPF_MAP_TYPE_PROG_ARRAY/) — Accessed 2026-06-02
**Confidence**: High
**Verification**: [eBPF Docs — Tail calls](https://docs.ebpf.io/linux/concepts/tail-calls/); [Cloudflare — "Assembly within! BPF tail calls"](https://blog.cloudflare.com/assembly-within-bpf-tail-calls-on-x86-and-arm/).
**Analysis**: A root XDP program attached to `lo` could `bpf_tail_call`
into the service-map program; that program, on a miss, `bpf_tail_call`s into
the reverse-NAT program. aya 0.13.x **does** ship `ProgramArray` (userspace)
and `ProgramArray` (kernel-side) typed wrappers per `aya-rs-usage` §A.1 — so
unlike Option A, this is within aya's native surface.

### Finding 4.2: Tail-call control-transfer semantics are awkward for "fall-through to next stage"

**Evidence**: Because a tail call does **not** return, the chaining must be
explicit: each stage, on its "not my traffic" path, must itself
`bpf_tail_call` to the next index (there is no automatic "the dispatcher
runs the next one"). The final stage returns the verdict. A missed
`bpf_tail_call` (empty slot) falls through to the instruction after the
helper — so the program must handle "tail call target absent" as a verdict
path.
**Source**: [eBPF Docs — `bpf_tail_call`](https://docs.ebpf.io/linux/helper-function/bpf_tail_call/) — Accessed 2026-06-02
**Confidence**: High
**Verification**: [Cilium BPF Architecture — tail-call usage](https://docs.cilium.io/en/latest/bpf/architecture/).
**Analysis**: This re-implements, by hand, exactly the chaining the
dispatcher (Option A) or a merged program (Option B) gives for free — but
with the added burden of a `PROG_ARRAY` to populate from userspace, a root
program to author and attach, and stack-budget reduction (512B → 256B when
tail calls are present, per Finding 3.3). It also adds a per-packet
indirect-call cost vs the merged program's straight-line fall-through.

### Finding 4.3: Tail calls add stack-budget and per-packet cost the merged program avoids

**Evidence**: "Without tailcalls a total stack of 512 bytes is allowed, with
tail-calls only a total stack size of 256 bytes is allowed." The header-
parse + NAT-rewrite paths use packet-scratch on the stack; halving the
budget is a real constraint.
**Source**: [Cilium BPF Architecture](https://docs.cilium.io/en/latest/bpf/architecture/) / eBPF Docs tail-call stack note — Accessed 2026-06-02
**Confidence**: High
**Verification**: same eBPF Docs tail-call page; consistent across both.
**Analysis**: For *two* statically-known stages, tail calls are strictly
more complex than Option B (merge) with no compensating benefit — the
indirection only pays off when stages are *dynamically* swappable or
*numerous* (Cilium's dozens of tail-call stages). Overdrive has two fixed
stages.

**Option C verdict**: aya-native (unlike Option A) and kernel-portable, but
strictly dominated by Option B for two fixed stages — more moving parts
(root program + PROG_ARRAY + userspace population), tighter stack budget,
per-packet indirect-call cost, and the same "author the chaining by hand"
burden. **Only justified if the project anticipates many dynamically-
composed XDP stages** (it does not, at Phase 1). Not recommended for this
fix.

## Section 5 — Option D: Split hooks across layers (XDP ingress + TC/`tcx`)

### Finding 5.1: XDP and TC/`tcx` are different hooks on the same netdev — no XDP-slot collision

**Evidence**: "XDP and TC/TCX operate at different layers of the Linux
networking stack, allowing them to coexist on the same interface without
collision." "For networking facing devices the tc ingress hook can be
coupled with the XDP hook." TC runs after the driver-stage XDP hook, before
L3. `tcx` (kernel ≥ 6.6) "provides a dedicated, qdisc-less extension point …
safe ownership, explicit ordering, revision-aware updates, and coexistence
with classic TC."
**Source**: [Cilium — eBPF Introduction / Architecture](https://docs.cilium.io/en/latest/network/ebpf/intro/) — Accessed 2026-06-02
**Confidence**: High
**Verification**: [eunomia — "Composable Traffic Control with TCX Links"](https://eunomia.dev/tutorials/50-tcx/); repo `aya-rs-usage` §B.2 (aya `SchedClassifier` + `qdisc_add_clsact`, TCX on ≥6.6, legacy netlink + `clsact` below).
**Analysis**: Putting one Overdrive stage on the XDP hook and the other on
the TC hook of `lo` sidesteps the single-XDP-slot constraint entirely:
forward path stays on XDP ingress; reverse path moves to TC. **No `EBUSY`
because they are not competing for the same hook.**

### Finding 5.2: But the project *already moved reverse-NAT off TC onto XDP* (ADR-0045) — this is a partial reversal

**Evidence** (repo): `xdp_reverse_nat.rs` L33-36 + the constructor docstring
(L316-320) state reverse-NAT was deliberately moved from a pre-pivot
`tc_reverse_nat` *egress* attach to XDP at the backend-facing veth ingress,
"replacing the pre-pivot `tc_reverse_nat` egress attach." The reverse-NAT
program relies on `bpf_redirect` and `bpf_fib_lookup` in their **XDP** forms;
ADR-0045 specifically notes `bpf_redirect_neigh` is TC-only and rejected on
XDP, and the program was written to the XDP helper surface.
**Source**: repo `crates/overdrive-bpf/src/programs/xdp_reverse_nat.rs`, ADR-0045 — Cross-referenced 2026-06-02
**Confidence**: High
**Verification**: constructor docstring step 6 (lib.rs L316-320).
**Analysis**: Moving reverse-NAT back to TC would **undo ADR-0045** and
require re-porting the program to the TC helper surface (`TcContext`,
`TC_ACT_*` verdicts, `bpf_redirect_neigh` availability) — a non-trivial
rewrite, and a reversal of a landed architectural decision. The *forward*
program is XDP-shaped (`XDP_TX`/`bpf_redirect`) and cannot trivially move to
TC either. This option is more disruptive than it first appears: it is not
"flip a flag," it is "re-architect one half of the dataplane back to TC."

### Finding 5.3: TC-hook reverse-NAT on `lo` has its own correctness questions

**Evidence/Analysis** (interpretation): The forward path uses `XDP_TX`/
`bpf_redirect` which bounce/redirect at the driver stage; a TC-egress
reverse-NAT sees packets *after* L3 routing. On `lo` specifically, the
packet path (loopback delivers via the kernel's loopback xmit, not a real
driver) interacts differently with XDP-generic vs TC than on a real NIC.
Whether the forward XDP stage's `XDP_TX` output is even *seen* by a TC hook
on the same `lo` depends on the exact redirect target. This needs Tier 3
confirmation on `lo` before relying on it.
**Confidence**: Low (loopback + XDP-generic + TC interaction is subtle; not
independently sourced for the `lo` case — see Gap G-3)
**Verification**: none direct; flagged as a gap.

**Option D verdict**: Architecturally legitimate and the "Cilium-style"
answer (XDP ingress + TC/`tcx` egress) — and it does dodge the XDP-slot
collision. **But** for Overdrive it means reversing ADR-0045 and re-porting
the reverse-NAT program to the TC helper surface, with unresolved
loopback-path correctness questions. High disruption, contradicts a landed
decision. **Not recommended** unless the team wants to revisit the
XDP-vs-TC split holistically.

## Section 4 — Option C: Tail calls (`BPF_MAP_TYPE_PROG_ARRAY`)

_(placeholder)_

## Section 5 — Option D: Split hooks across layers (XDP ingress + TC/tcx)

_(placeholder)_

## Section 6 — Option E: Is `lo` the right target? (dedicated veth/dummy)

### Finding 6.1: Generic (SKB-mode) XDP on `lo`/veth has real correctness limitations, not just perf

**Evidence**: "For virtual devices like veth devices XDP is unsuitable since
the kernel operates solely on an skb here and generic XDP has a few
limitations where it does not operate with cloned skb's. The TCP/IP stack
heavily uses cloned skb's for data segments for retransmission where the
generic XDP hook would simply get bypassed, and generic XDP needs to
linearize the entire skb resulting in heavily degraded performance." "Cilium
only supports native XDP on user side. Generic XDP [is] only utilized for CI
purpose."
**Source**: [cilium/cilium #12910 — "Why not use veth native xdp"](https://github.com/cilium/cilium/issues/12910) / Cilium tuning guidance — Accessed 2026-06-02
**Confidence**: High
**Verification**: [Cilium Tuning Guide](https://docs.cilium.io/en/stable/operations/performance/tuning/) (native-XDP requirement); [Cilium BPF program types](https://docs.cilium.io/en/latest/bpf/progtypes/).
**Analysis**: This is a deeper problem than the `EBUSY` collision. `lo` has
no native XDP driver, so Overdrive's loopback attach is **always**
generic/SKB-mode — and generic XDP on loopback can be *bypassed entirely*
for cloned skbs (the common TCP retransmit / segmentation path). A
single-node dataplane attached to `lo` in generic mode may silently miss
traffic regardless of how the two-program collision is resolved. The
`EBUSY` is the *first* symptom of "`lo` is the wrong target," not the only
one.

### Finding 6.2: Comparable XDP load balancers do NOT attach to `lo`; they use real/virtual NICs with native XDP

**Evidence**: Katran (Meta's XDP L4LB) and Cilium attach XDP to real NICs
(or veth with native XDP where the driver supports it), never loopback.
Cilium's XDP load-balancing path is documented as a native-XDP feature on
the physical NIC facing the network. Tutorials that demonstrate XDP LB on a
single host use a dedicated veth pair (with native veth XDP, kernel ≥ 5.x)
or a real interface — not `lo`.
**Source**: [Cilium 1.8 XDP Load Balancing announcement](https://cilium.io/blog/2020/06/22/cilium-18/) — Accessed 2026-06-02
**Confidence**: Medium-High
**Verification**: [Cilium L4 Load Balancer use-case](https://cilium.io/use-cases/load-balancer/); [eunomia-bpf XDP LB tutorial (veth-based single-host)](https://github.com/eunomia-bpf/bpf-developer-tutorial/blob/main/src/42-xdp-loadbalancer/README.md); repo's own ADR-0043 three-iface test topology uses **veth pairs**, not `lo`.
**Analysis**: The project's *test* topology already proves the point — the
Tier 3 integration tests (`ThreeIfaceTopology`, ADR-0043) build veth pairs
and attach there, **not** to `lo`. The single-node *runtime* default
(`DataplaneConfig::loopback()`) is the odd one out: it points at an
interface the tests never use and that XDP serves poorly.

### Finding 6.3: A dedicated veth/dummy pair preserves the two-distinct-iface design AND dodges multiprog

**Evidence/Analysis** (interpretation, repo-grounded): If single-node
Overdrive provisioned a dedicated veth pair (e.g. `ov-client` ↔ `ov-backend`)
at boot — the same shape the Tier 3 tests already construct — then
`client_iface` and `backend_iface` map to **two distinct netdevs** again.
The existing two-program / two-attach code path works unchanged: one XDP
program per hook, no collision, no `EBUSY`. This requires zero kernel-side
change and zero new BPF mechanism; it is a *deployment/config* change
(provision the veth pair, point the config at it) plus the routing/plumbing
to steer single-node service traffic through the client side. Native veth
XDP (kernel ≥ 5.x with the veth driver's native XDP support) also restores
non-degraded XDP semantics that `lo` cannot offer (Finding 6.1).
**Confidence**: Medium (the *collision fix* is high-confidence — two ifaces,
two hooks, no `EBUSY`; the *traffic-steering* plumbing for single-node is a
design task needing its own validation — see Gap G-4)
**Verification**: consistent with Finding 1.1 (one program per hook) and the
repo's ADR-0043 veth topology precedent.

**Option E verdict**: This is the option that treats the *actual* root
cause — `lo` is the wrong attach target for an XDP dataplane, both for the
collision (two programs, one hook) and for correctness (generic XDP on
loopback bypasses cloned skbs). A dedicated veth pair restores the
two-distinct-iface invariant the production code already assumes, requires
**no kernel-side or BPF-mechanism change**, matches the project's own Tier 3
topology, and aligns with how every comparable XDP LB deploys. The cost is
single-node deployment plumbing (provision the pair, steer traffic), not
dataplane surgery. **Strong candidate — arguably the most architecturally
honest fix.**

## Section 7 — Recommendation Matrix

| Axis | A. libxdp dispatcher | B. Merge into one XDP program | C. Tail calls (PROG_ARRAY) | D. XDP + TC/tcx split | E. Dedicated veth/dummy |
|---|---|---|---|---|---|
| Kernel floor | 5.10 (full freplace); 5.6 (all-at-once) | any XDP-capable | any XDP-capable | TC ≥4.1 / tcx ≥6.6 | any (veth XDP ≥5.x for native) |
| aya 0.13.x native? | **No** — `Extension` primitive only; no dispatcher. Hand-roll ABI or FFI libxdp | Yes — single `#[xdp]` | Yes — `ProgramArray` shipped | Yes — `SchedClassifier`/tcx shipped | Yes — no BPF change at all |
| Impl size / risk | **High** (multi-week; libxdp dispatcher ABI or C FFI dep) | **Low–Med** (one-time kernel-side merge + re-baseline) | Med (root prog + PROG_ARRAY + userspace populate) | **High** (reverse ADR-0045; re-port to TC surface) | **Low** (deploy/config plumbing; no BPF change) |
| Verifier-budget impact | Per-program unchanged (dispatcher tiny) | Concatenated; measured by Tier-4 gate; bounded | Stack 512→256B; per-tail-call cost | Per-program unchanged | None |
| Cleanup / leak | Dispatcher + N freplace links + bpffs pins | One link (simpler than today) | Root link + PROG_ARRAY entries | XDP link + TC/tcx link | Two links + veth lifecycle |
| Regresses two-NIC path? | No (also usable there) | No (attach merged prog to each iface) | No | **Risk** — TC re-port affects both | No (restores the design) |
| Fixes `lo` correctness (cloned-skb bypass)? | **No** | **No** | **No** | Partially (TC handles skb) | **Yes** |
| All-Rust/aya posture | Breaks (FFI) or heavy hand-roll | Keeps | Keeps | Keeps | Keeps |

**Reading the matrix**: Options A, C, D each pay a real cost (hand-rolled
dispatcher / extra mechanism / architectural reversal) to keep two *separate*
programs on one hook — but none of them fixes the deeper `lo` correctness
problem (Finding 6.1). Options B and E are the low-risk fixes; E uniquely
fixes the loopback correctness issue too.

## Recommendation

**Ranked for Overdrive's Phase-1 single-node-in-scope constraint:**

1. **Option E (dedicated veth pair) — recommended primary.** It is the only
   option that fixes the *actual* root cause: `lo` is the wrong attach
   target for an XDP dataplane (single XDP slot → `EBUSY`, *and* generic
   XDP on loopback can bypass cloned skbs → silent traffic miss). A
   dedicated `ov-client`↔`ov-backend` veth pair restores the
   two-distinct-iface invariant the production code already assumes, needs
   **no kernel-side or BPF-mechanism change**, matches the project's own
   ADR-0043 Tier-3 veth topology, and mirrors how Cilium/Katran deploy. The
   work is single-node deployment plumbing (provision the pair at boot,
   steer service traffic through the client side), not dataplane surgery.
   The remaining open question is the traffic-steering plumbing (Gap G-4) —
   resolvable as a focused design task.

2. **Option B (merge the two XDP programs) — recommended fallback / or
   complementary.** If single-node *must* attach to a single given iface
   (including a single veth, or even `lo` accepting its limits), merging the
   two stages into one staged `#[xdp]` entry is the lowest-mechanism way to
   put both behaviours on one hook. The two programs already share the
   sanity prologue and header parse and already early-return `XDP_PASS`, so
   the merge is mechanical; the only cost is a Tier-4 verifier re-baseline
   (bounded, gated). It also *simplifies* the attach path (one program,
   attached to each configured iface — 1 for single-node, 2 for two-NIC),
   removing the special case. **B and E compose**: provision a veth pair
   (E) AND attach one merged program (B) for the cleanest single + two-NIC
   unification.

3. **Option C (tail calls) — not recommended.** aya-native and portable,
   but strictly dominated by B for two fixed stages: more moving parts,
   halved stack budget, per-packet indirect-call cost, same hand-authored
   chaining. Only justified if Overdrive expects *many dynamically composed*
   XDP stages — it does not at Phase 1.

4. **Option D (XDP + TC/tcx split) — not recommended.** Legitimate
   Cilium-style architecture, but for Overdrive it means **reversing the
   landed ADR-0045** (reverse-NAT moved TC→XDP deliberately) and re-porting
   the reverse-NAT program to the TC helper surface, with unresolved
   loopback-path correctness questions. High disruption against a decided
   architecture.

5. **Option A (libxdp dispatcher) — not recommended for this fix.** The
   canonical answer for N-program XDP chaining, but aya 0.13.x ships **no
   dispatcher** (only the `Extension`/freplace primitive). Realising it
   means a multi-week hand-roll of the libxdp dispatcher ABI or a C-library
   FFI dependency — disproportionate to a two-program single-node default,
   and it still doesn't fix the `lo` correctness problem. Revisit only if
   the project later needs operator-pluggable XDP program chains.

**Bottom line**: Fix the target, not just the symptom. **Provision a
dedicated veth pair for single-node (E)**, and **optionally merge the two
programs (B)** to unify the attach path across single-node and two-NIC.
Together they remove `EBUSY`, restore correct XDP semantics, keep the
all-Rust/aya posture, and require no new kernel mechanism.

## Decision-Ready Summary

- **Root cause**: `DataplaneConfig::loopback()` points both `client_iface`
  and `backend_iface` at `lo`; the kernel allows exactly one program on a
  netdev's XDP hook, so the second `attach` returns `EBUSY`. `bpf_link` does
  **not** change this (one owner per hook); `REPLACE`/`UPDATE_IF_NOEXIST`
  manage the single slot, they don't add a second program.
- **Deeper issue**: `lo` has no native XDP → forced generic mode, which can
  *bypass* cloned skbs (TCP retransmit path) — a single-node `lo` dataplane
  may silently miss traffic even after the collision is fixed.
- **Five options, two viable for Phase 1**:
  - **E — dedicated veth pair (PRIMARY)**: no BPF change; restores
    two-iface design; fixes both collision and loopback correctness; matches
    ADR-0043 test topology. Cost: single-node deploy plumbing (Gap G-4).
  - **B — merge two programs into one staged XDP entry (FALLBACK /
    COMPLEMENT)**: lowest new-mechanism cost; mechanical (shared prologue,
    disjoint maps, both early-return `XDP_PASS`); cost = Tier-4 verifier
    re-baseline. Unifies single + two-NIC attach.
  - C/D/A: dominated / disruptive / over-engineered respectively — not
    recommended now.
- **Recommended move**: E (and optionally B). Neither needs a kernel-floor
  change, an FFI dependency, or a reversal of ADR-0045.
- **Before implementing**: resolve Gap G-4 (single-node traffic steering
  through the veth client side) as a focused design task; if B is chosen,
  capture a fresh Tier-4 verifier baseline for the merged program.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| libxdp README.org (xdp-tools) | github.com/xdp-project | High (1.0) | Official libxdp docs | 2026-06-02 | Y |
| libxdp(3) man pages (Debian/Ubuntu) | manpages.debian.org / .ubuntu.com | High (1.0) | Official man page | 2026-06-02 | Y |
| eBPF Docs — BPF_PROG_TYPE_XDP | docs.ebpf.io | High (1.0) | Curated kernel reference (isovalent) | 2026-06-02 | Y |
| eBPF Docs — bpf_xdp_attach (libbpf) | docs.ebpf.io | High (1.0) | Curated kernel reference | 2026-06-02 | Y |
| eBPF Docs — BPF_PROG_TYPE_EXT | docs.ebpf.io | High (1.0) | Curated kernel reference | 2026-06-02 | Y |
| eBPF Docs — BPF_MAP_TYPE_PROG_ARRAY / tail calls / bpf_tail_call | docs.ebpf.io | High (1.0) | Curated kernel reference | 2026-06-02 | Y |
| aya 0.13.1 `Extension` docs.rs | docs.rs | High (1.0) | Official aya API docs | 2026-06-02 | Y |
| aya 0.13.1 `programs` index docs.rs | docs.rs | High (1.0) | Official aya API docs | 2026-06-02 | Y |
| netdev — freplace multi-attach patch series | mail-archive.com/netdev | High (1.0) | Kernel mailing list (primary) | 2026-06-02 | Y |
| Cilium — eBPF intro / architecture / progtypes / tuning | docs.cilium.io / cilium.io | High (1.0) | OSS project docs | 2026-06-02 | Y |
| cilium/cilium #12910 (veth/generic-XDP limits) | github.com/cilium | High (1.0) | Maintainer discussion | 2026-06-02 | Y |
| Cloudflare — BPF tail calls on x86/ARM | blog.cloudflare.com | High (1.0) | Industry-leader engineering | 2026-06-02 | Y |
| Toke Høiland-Jørgensen — XDP multiprog (LPC) | lpc.events | High (1.0) | Conference (dispatcher author) | 2026-06-02 | Y |
| eunomia — TCX links tutorial | eunomia.dev | Medium-High (0.8) | Practitioner tutorial | 2026-06-02 | Cross-ref to Cilium |
| Repo: lib.rs, dataplane_config.rs, xdp_*.rs, ADR-0043/0045, aya-rs-usage research | (project-internal) | — | Primary source code | 2026-06-02 | Y |

Reputation: High: 13 of 14 external (93%) | Medium-High: 1 (7%) | Avg ≈ 0.99. Every major claim cross-referenced ≥ 2 independent sources.

## Knowledge Gaps

### Gap G-1: No single kernel-doc page quoted verbatim for "bpf_link XDP allows only one owner per hook"
**Issue**: The claim (Finding 1.4) is corroborated across the eBPF Docs,
libxdp README (dispatcher exists *because* of the single slot), and the
cilium/ebpf discussion — but I did not retrieve a kernel.org page stating it
in one sentence (`docs.kernel.org/bpf/prog_xdp.html` 404'd; the XDP page
location has moved).
**Attempted**: kernel.org prog_xdp.html (404), af_xdp.html (no coverage),
WebSearch consensus.
**Recommendation**: For absolute certainty, read `net/core/dev.c`
`dev_xdp_attach` / `dev_xdp_install` in the kernel source — the single
`bpf_prog`/`bpf_link` slot per `xdp` attach-type is enforced there. Low risk;
the multi-source consensus is already conclusive for decision-making.

### Gap G-2: Merged-program verifier budget not measured on the actual programs
**Issue**: Option B's verifier-budget cost (Finding 3.3) is bounded *in
principle* but the concatenated instruction count of
`xdp_service_map_lookup` + `xdp_reverse_nat_lookup` (+ shared prologue once)
is not measured.
**Attempted**: read current per-program shapes; the Tier-4 baselines exist
under `perf-baseline/main/` but a merged baseline does not.
**Recommendation**: If B is chosen, capture
`bpf_prog_info.verified_insns` for the merged program via
`cargo verifier-regress` and confirm ≤ 50% of the 1M ceiling before relying
on it. This is implementation-time work, not a research blocker.

### Gap G-3: TC-hook reverse-NAT correctness on `lo` (Option D)
**Issue**: Whether a TC-egress reverse-NAT sees and correctly rewrites the
forward XDP stage's `XDP_TX`/`bpf_redirect` output on the *same* `lo` is
unconfirmed; the loopback path + generic-XDP + TC interaction is subtle.
**Attempted**: Cilium docs (general XDP/TC coexistence, not the `lo`
specific case).
**Recommendation**: Moot if D is rejected (recommended). If D is ever
revisited, a Tier-3 test on `lo` is mandatory before relying on it.

### Gap G-4: Single-node traffic-steering through a dedicated veth pair (Option E)
**Issue**: Option E provisions `ov-client`↔`ov-backend`, but *how*
single-node service traffic is steered through the client side (routing,
VIP plumbing, who originates the packets that hit `client_iface`) is a
design task this research did not scope.
**Attempted**: ADR-0043 (test topology) shows the veth + route + sysctl
shape the tests use; the runtime single-node steering is not yet specified.
**Recommendation**: A focused DESIGN dispatch (architect agent) to specify
the single-node veth provisioning + traffic-steering, reusing the
`ThreeIfaceTopology` / `threeiface_ips` plumbing from `overdrive-testing` as
the reference shape. This is the gating unknown for the recommended option.

## Full Citations

[1] xdp-project. "libxdp — README.org (multiprog, dispatcher, freplace, run priority, chain-call actions, kernel requirements)". github.com/xdp-project/xdp-tools. 2026. https://github.com/xdp-project/xdp-tools/blob/main/lib/libxdp/README.org. Accessed 2026-06-02.

[2] xdp-project. "libxdp — protocol.org (dispatcher config-section ABI)". github.com/xdp-project/xdp-tools. 2026. https://github.com/xdp-project/xdp-tools/blob/main/lib/libxdp/protocol.org. Accessed 2026-06-02.

[3] isovalent / eBPF Docs. "Program Type 'BPF_PROG_TYPE_XDP'". docs.ebpf.io. 2026. https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_XDP/. Accessed 2026-06-02.

[4] isovalent / eBPF Docs. "Libbpf userspace function 'bpf_xdp_attach'". docs.ebpf.io. 2026. https://docs.ebpf.io/ebpf-library/libbpf/userspace/bpf_xdp_attach/. Accessed 2026-06-02.

[5] isovalent / eBPF Docs. "Program Type 'BPF_PROG_TYPE_EXT'". docs.ebpf.io. 2026. https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_EXT/. Accessed 2026-06-02.

[6] isovalent / eBPF Docs. "Map Type 'BPF_MAP_TYPE_PROG_ARRAY' / Tail calls / Helper 'bpf_tail_call'". docs.ebpf.io. 2026. https://docs.ebpf.io/linux/concepts/tail-calls/. Accessed 2026-06-02.

[7] Aya Project. "aya 0.13.1 — `aya::programs::extension::Extension`". docs.rs. 2025. https://docs.rs/aya/0.13.1/aya/programs/extension/struct.Extension.html. Accessed 2026-06-02.

[8] Aya Project. "aya 0.13.1 — `aya::programs` index (program types, MultiProgLink/MultiProgram)". docs.rs. 2025. https://docs.rs/aya/0.13.1/aya/programs/index.html. Accessed 2026-06-02.

[9] netdev (linux kernel networking list). "[PATCH bpf-next] bpf: Support multi-attach for freplace programs (freplace/BPF_PROG_TYPE_EXT history)". mail-archive.com/netdev. 2026. https://www.mail-archive.com/netdev@vger.kernel.org/msg356216.html. Accessed 2026-06-02.

[10] Cilium Authors. "eBPF Introduction / BPF Architecture / Program Types / Tuning Guide". docs.cilium.io. 2026. https://docs.cilium.io/en/latest/network/ebpf/intro/. Accessed 2026-06-02.

[11] Cilium Authors. "Why not use veth native xdp for network policy (Issue #12910 — generic-XDP cloned-skb limits)". github.com/cilium/cilium. 2020. https://github.com/cilium/cilium/issues/12910. Accessed 2026-06-02.

[12] Cilium. "Cilium 1.8: XDP Load Balancing …". cilium.io. 2020. https://cilium.io/blog/2020/06/22/cilium-18/. Accessed 2026-06-02.

[13] Cloudflare. "Assembly within! BPF tail calls on x86 and ARM". blog.cloudflare.com. 2021. https://blog.cloudflare.com/assembly-within-bpf-tail-calls-on-x86-and-arm/. Accessed 2026-06-02.

[14] Høiland-Jørgensen, Toke. "XDP multiprog (Linux Plumbers Conference, Networking & BPF Summit)". lpc.events. 2020. https://lpc.events/event/7/contributions/671/attachments/561/992/xdp-multiprog.pdf. Accessed 2026-06-02.

[15] eunomia. "eBPF Tutorial 50: Composable Traffic Control with TCX Links". eunomia.dev. 2024. https://eunomia.dev/tutorials/50-tcx/. Accessed 2026-06-02.

[16] Debian / Ubuntu. "libxdp(3) man page". manpages.debian.org. 2023. https://manpages.debian.org/bookworm/libxdp-dev/libxdp.3.en.html. Accessed 2026-06-02.

[17] eunomia-bpf. "bpf-developer-tutorial — 42-xdp-loadbalancer (veth-based single-host XDP LB)". github.com/eunomia-bpf. 2024. https://github.com/eunomia-bpf/bpf-developer-tutorial/blob/main/src/42-xdp-loadbalancer/README.md. Accessed 2026-06-02.

[18] Overdrive Project. "crates/overdrive-dataplane/src/lib.rs (EbpfDataplane::new_with_pin_dir), crates/overdrive-control-plane/src/dataplane_config.rs (loopback()), crates/overdrive-bpf/src/programs/{xdp_service_map,xdp_reverse_nat}.rs, ADR-0043, ADR-0045, docs/research/dataplane/aya-rs-usage-comprehensive-research.md". 2026. (project-internal). Cross-referenced 2026-06-02.

## Research Metadata

Duration: ~45 turns | Examined: 18 sources (14 external + repo code) | Cited: 18 | Cross-refs: every major claim ≥ 2 | Confidence: High (kernel model, option mechanics, kernel floors); Medium (single-node steering plumbing for E, merged-program budget for B — flagged as gaps) | Output: docs/research/dataplane/xdp-multiprog-same-iface-aya-research.md
