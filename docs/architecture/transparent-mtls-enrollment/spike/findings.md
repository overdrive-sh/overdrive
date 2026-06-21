# Spike findings — Probe A: kernel-side orig-dst recovery for the `cgroup/connect4`-redirected outbound mTLS leg (transparent-mtls-enrollment, GH #236)

**nw-spike Phase 1 (PROBE) — throwaway, real-kernel.** Status: IN PROGRESS.

The probe is a **self-contained, gitignored aya-rs Cargo workspace** at
`spike-scratch/increment-a/` (`.gitignore:29`) — it drags in NONE of the
production bpf-build chain (no `overdrive_bpf.o`, no SERVICE_MAP HoM, no
`overdrive-*` deps). Two members:

- `spike-scratch/increment-a/ebpf/` — standalone `#![no_std] #![no_main]`
  aya-ebpf crate (own `[workspace]` root, target `bpfel-unknown-none`,
  `aya-ebpf 0.1.1` — the production pin). One file `src/main.rs` carries
  both kernel-side programs (`#[cgroup_sock_addr(connect4)]` +
  `#[cgroup_sockopt(getsockopt)]`) and six `HashMap`s (no HoM — the probe
  needs only plain maps, fully supported by aya 0.13.x).
- `spike-scratch/increment-a/loader/` — userspace `aya 0.13` driver
  (`EbpfLoader::load_file` + `aya::programs::{CgroupSockAddr, CgroupSockopt}`).
  Creates a test cgroup, attaches both programs, programs leg-F, spawns a
  workload child that `connect()`s a real peer, accepts on leg-F, calls
  `getsockopt(SO_ORIGINAL_DST)`, reports, tears down (`cgroup.kill` + rmdir).

Build (inside Lima, the recipe `xtask bpf-build` uses):
`rustup run nightly cargo build --release -Z build-std=core` for the ebpf
(target pinned in `ebpf/.cargo/config.toml` + `bpf-linker`); plain
`cargo build --release` for the loader. Run as root via `cargo xtask lima
run -- <bin> agent` (cgroup create + BPF attach need root). The loader takes
the built ELF path via `PROBE_EBPF_OBJECT`.

NOTHING was written in C / libbpf / `vmlinux.h` (the corrected failure mode);
the probe is 100% aya-rs Rust on both sides. NOTHING was written under
`crates/` (the second corrected failure mode) — the spike is entirely inside
gitignored `spike-scratch/`. Nothing committed; no `overdrive-*` production
API touched; the Phase-2 promotion gate is NOT run by this agent.

---

## The ONE assumption under test (Probe A)

> On the running `overdrive` Lima kernel, can a node-agent recover a workload's
> ORIGINAL outbound destination after a `cgroup/connect4` BPF program has
> redirected the connection to the agent's local proxy leg (leg-F) — using a
> kernel-side cookie/4-tuple-stash + `cgroup/getsockopt(SO_ORIGINAL_DST)`
> mechanism, with NO pre-programmed per-destination map entry?

**Why it matters:** today the production worker
(`crates/overdrive-worker/src/mtls_intercept_worker.rs`) cannot observe the
real peer from the connection alone — the `connect4` sockaddr rewrite is lossy,
so a test-only `program_declared_peer_redirect` seam SUPPLIES `real_peer`. If
Probe A WORKS, the agent recovers orig-dst for ANY destination, retiring that
seam and enabling the enrollment-based interception model (#236). If it
DOESN'T, that points to Probe B (inbound-style TPROXY + `getsockname`, already
proven for the inbound half in
`docs/feature/transparent-mtls-host-socket/spike/findings-inbound-intercept.md`).

### The crux within the crux

The workload's `connect()` socket (where `connect4` fires + rewrites dst → leg-F)
and the agent's `accept()`-ed leg-F socket (where the agent wants orig-dst) are
**two different sockets**. `bpf_sk_storage` is per-socket, so a stash on the
connect socket is NOT directly readable from the accept socket. The real
question: **by what key does the AGENT's accepted leg-F socket recover the
WORKLOAD's dialed destination?**

## Hypothesis / Prediction / Falsification

- **Hypothesis:** the agent's accepted leg-F socket recovers the workload's real
  dialed `(ip,port)` via a BPF `cgroup/getsockopt(SO_ORIGINAL_DST)` mechanism
  keyed on a correlation the agent side can reconstruct, with NO pre-programmed
  per-peer entry.
- **Predicted:** `getsockopt(SOL_IP, SO_ORIGINAL_DST)` on the agent's accepted
  socket returns the REAL peer `(ip,port)`, not leg-F.
- **Falsified if:** it returns leg-F (correlation not wired), or the
  `cgroup/getsockopt` program does not fire / cannot write optval on this
  kernel, or no correlation key reliably links the two sockets, or a
  lifetime/race issue corrupts the value.

---

## Step 0 — Kernel version (pin the verdict)

`uname -r -m`: **`7.0.0-22-generic aarch64`** (Ubuntu 26.04; ≥ the pinned 6.18
floor, ADR-0068). Measured via `cargo xtask lima run --no-sudo -- uname -r -m`.
The eBPF object is a valid `ELF 64-bit LSB relocatable, eBPF` (3776 bytes,
sections `cgroup/connect4`, `cgroup/getsockopt`, `maps`, `.text`, `.relcgroup`;
both program symbols present — NOT a 1.3 KB stub).

## Binary verdict: **DOESN'T-WORK**

The `cgroup/connect4` → `cgroup/getsockopt(SO_ORIGINAL_DST)` mechanism does
**NOT** recover the workload's original outbound destination on the agent's
accepted leg-F socket. Two independent walls, either of which alone is fatal:

1. **No correlation key links the two sockets without a pre-programmed entry.**
   `connect4` fires *before* the kernel binds the ephemeral source port, so it
   cannot key the stash by the connection's source 4-tuple. The agent's
   accepted socket *can* read that source 4-tuple, but the stash slot for it is
   empty → MISS (`verdict=3`: tuple-miss, witness-present). The socket-cookie
   alternative is **verifier-FORBIDDEN** in `cgroup/getsockopt` (see below).
2. **The kernel's native `SO_ORIGINAL_DST` returns `ENOENT` (errno 2)** for a
   `cgroup/connect4` sockaddr-rewrite — because that rewrite is NOT a netfilter
   DNAT and leaves no conntrack original-tuple record. `SO_ORIGINAL_DST` reads
   conntrack; there is nothing to read.

And a third structural wrinkle that compounds (2): the **`cgroup/getsockopt`
hook only fires for getsockopt calls made by a process IN the attached
cgroup** — the AGENT (which calls `getsockopt` on the accepted socket) is, by
the interception model's own F5 exemption, *outside* the workload cgroup, so
the hook never even runs on the agent's recovery call.

## Recovered-vs-expected evidence (real numbers, real `getsockopt` output)

Canonical run (kernel 7.0.0-22-generic aarch64), agent reporting:

```
kernel real-peer (expected orig dst) = 127.0.0.1:33303     <- what we want back
leg-F (redirect target)              = 127.0.0.1:36723
connect4 witness LAST_REAL_DST[0]    = 127.0.0.1:33303     <- connect4 captured it ✓
agent: getsockopt(SO_ORIGINAL_DST)   rc=-1 errno=2 (No such file or directory)
getsockopt verdict code = Some(3)    (tuple MISS, witness present)
VERDICT: DOESN'T-WORK — no orig-dst recoverable on the accepted socket
```

- **Expected** orig-dst = `127.0.0.1:33303` (the real peer the workload dialed).
- **Recovered** = nothing — `getsockopt(SOL_IP, SO_ORIGINAL_DST)` returned
  `rc=-1, errno=2 (ENOENT)`, no `sockaddr_in` written.
- **connect4 DID fire and capture the right value**: the witness map
  `LAST_REAL_DST[0]` holds `127.0.0.1:33303` exactly. Confirmed independently
  via `bpftool map dump`: `key: 00 00 00 00  value: 01 00 00 7f <port> 00 00`
  (`0x7f000001` host-order = 127.0.0.1). The value IS captured globally; it is
  just not correlatable to the agent's accepted socket.
- **bpftool ground-truth** (independent of the loader's self-report): both
  programs are attached to the test cgroup —
  `60925 cgroup_inet4_connect … probe_connect4_` and
  `60926 cgroup_getsockopt … probe_getsockop` (AttachType `cgroup_getsockopt`,
  flags `multi`). `GETSOCKOPT_SEEN` map: **`Found 0 elements`** when only the
  out-of-cgroup agent called getsockopt — confirming the hook genuinely never
  fired for the agent's recovery call, despite a correct attach.

## Correlation key/mechanism that worked (or the wrinkle)

**None worked.** Each candidate and why it fails on this kernel:

| Candidate | Result | Why |
|---|---|---|
| `bpf_get_socket_cookie` stash (connect socket) → read back (accept socket) | **verifier-FORBIDDEN** | `cgroup/getsockopt` cannot call helper #46. Empirically reproduced: verifier log `program of this type cannot use helper bpf_get_socket_cookie#46` (EINVAL at `prog.load()`). Even if allowed, cookies are per-socket → the two sockets have different cookies anyway. |
| source-4-tuple stash (connect4) → lookup (getsockopt) | **MISS** | `connect4` fires before the ephemeral source port is bound; it cannot write the key the accept side would read. `verdict=3` every run. |
| kernel-native `SO_ORIGINAL_DST` pass-through | **ENOENT (errno 2)** | A `cgroup/connect4` sockaddr rewrite is not a conntrack DNAT; no original-tuple record exists for conntrack's `getorigdst` to return. Tested on BOTH the agent's accept socket AND the workload's own connect socket — both `errno=2`. |
| `cgroup/getsockopt` writing optval from the witness | **hook doesn't fire on the recovery call** | The hook fires only for an in-cgroup caller. The agent is out-of-cgroup by design (F5); when the in-cgroup workload calls getsockopt the hook DOES fire (`total=2, optname=80`), proving the hook works — but the recovery has to happen on the *agent's* out-of-cgroup socket, where it never runs. |

The crux "by what key does the AGENT's accepted leg-F socket recover the
WORKLOAD's dialed destination?" has, on this kernel, **no answer in the
connect4+getsockopt family** without a pre-programmed per-destination entry —
which is exactly what the probe was testing the ability to AVOID.

## Edge cases / kernel surprises

- **The `cgroup/getsockopt` hook is gated on the CALLING task's cgroup, not the
  socket's connection origin.** Verified three ways: (a) agent out-of-cgroup →
  `GETSOCKOPT_SEEN` empty (bpftool: `Found 0 elements`); (b) agent joined to the
  cgroup before `accept()` → still empty (the accepted socket's getsockopt by
  the agent still didn't fire — falsifies "socket-cgroup membership" as the
  gate); (c) the in-cgroup *workload* calling getsockopt on its own socket →
  `total=2, last(level=0, optname=80)`. The hook works; it is just scoped to the
  wrong actor for this recovery shape.
- **`bpf-linker` emits a benign `dlopen failed` warning** (`libLLVM-22-rust-…
  .so: dlopen failed`) on this Lima nightly and STILL produces a valid ELF —
  the same warning the production `cargo xtask bpf-build` emits. The artifact
  lands under `$CARGO_TARGET_DIR` (`/home/marcus.guest/.cargo-target-lima/`),
  NOT the crate-local `target/`. (Cost a detour; recorded so the next probe
  doesn't repeat it.)
- **The sockaddr rewrite is "real" at the socket layer**: the workload's
  connect socket reports its connected peer as leg-F (the rewritten dest), not
  the dialed peer — which is precisely why a conntrack-based `SO_ORIGINAL_DST`
  has nothing to recover.

## Design implications for #236

- The current production seam (`mtls_intercept_worker.rs` test-only
  `program_declared_peer_redirect` supplying `real_peer`) **cannot be retired
  via connect4+getsockopt(SO_ORIGINAL_DST)** on the appliance kernel. The
  mechanism is a dead end for kernel-mediated orig-dst recovery.
- The viable kernel-native recovery shape is the **inbound-style TPROXY +
  `getsockname`** path (Probe B), already proven for the inbound half in
  `docs/feature/transparent-mtls-host-socket/spike/findings-inbound-intercept.md`.
  With TPROXY the redirect preserves the original destination as the socket's
  *local* address on the agent's accepting socket, recoverable via a plain
  `getsockname` (no conntrack, no cross-socket correlation, no cgroup-scope
  mismatch). The outbound leg should mirror that mechanism, not connect4.
- If a connect4-family approach is ever revisited, the ONLY workable key is a
  **pre-programmed per-(source-tuple-after-bind) entry written from a LATER
  hook** (e.g. a `sockops`/`sk_lookup` stage that fires after the ephemeral
  port is bound) — i.e. exactly the "pre-programmed per-destination map entry"
  this probe was trying to avoid. That re-introduces the lookup-table the
  enrollment model wanted to eliminate.

## One-line gate recommendation

**PIVOT to Probe B (TPROXY + `getsockname`).** Probe A is a confirmed
DOESN'T-WORK: connect4+`getsockopt(SO_ORIGINAL_DST)` cannot recover orig-dst on
the kernel (conntrack ENOENT + no cross-socket key + getsockopt hook scoped to
the wrong actor) — do NOT confirm it as the production recovery mechanism.
