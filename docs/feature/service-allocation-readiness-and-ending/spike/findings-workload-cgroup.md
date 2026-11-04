# Spike findings — workload cgroup for the listen hooks (V42)

Increment: `spike-scratch/workload-cgroup/`. Real Cloud Hypervisor guest boots on the metal host, kernel `7.0.0-38-generic`, `systemd-detect-virt: none`, Cloud Hypervisor v53.0.0. Raw output is in `spike-scratch/workload-cgroup/evidence/` (`raw-{a,b,c,c2,d,d-run1}.log`, where `raw-d-run1.log` and `summary-d-run1.txt` are the first of the two cost runs (arm D) and `raw-d.log` is the second; they agree within 0.2 ms. `summary-*.txt`, `sample-boots.txt`, `cleanup-confirmation.txt`).

**Gate: PROMOTE PR-2**, with two constraints pinned in D13: the listen programs live in the guest object `overdrive-init` already loads (never a second `Ebpf::load`), and the root programs stay link attachments (`BPF_F_ALLOW_MULTI`).

## Question

Production `overdrive-init` binds its own transport sockets before `READY` (TCP intake 61000 and 1,024 UDP slots at `workload_addr`). Does attaching the three listen programs to a child workload cgroup, with the workload placed into it at creation, report the workload's listeners and none of init's, keep the root programs effective, and stay within the cold-start budget?

## Method

- The real init is regenerated at build time by `gen_init.py` from `crates/overdrive-init/src` into a gitignored `out/gen*` directory on the metal host, with eight anchored patches. No copy of production source is committed. The run mode comes from the kernel parameter `ovdwc=`.
- BPF programs: `wc_listen` (sock_ops, `BPF_SOCK_OPS_TCP_LISTEN_CB`), `wc_post_bind4` / `wc_post_bind6` (protocol 17 only, return 1), built as a standalone object and linked into a byte-faithful copy of the guest object. `listen.rs`, `guest_capture.rs` and `tcp_correlation.rs` are reused by path from `spike-scratch/listen-hooks/bpf/`; the copies were diffed against `crates/overdrive-bpf/src/programs/` and are identical apart from the inactive `cfg(feature = "listen")` arm.
- Placement: `clone3` with `CLONE_INTO_CGROUP`. The child is created already in the workload cgroup, so no workload instruction runs elsewhere. A parent writing the pid into `cgroup.procs` has a window by construction; a `pre_exec` self-write moves the process after it exists. Neither was tested.

## Results

| Item | Verdict | Evidence |
|---|---|---|
| (a) 1000 boots, exact reports | **WORKS** | 1000/1000 exact (400 `single`, 300 `childonly`, 300 `parentchild`); 0 init-originated reports; counters match events, 0 drops; every workload process read `0::/workload` (`evidence/sample-boots.txt`). |
| (b) negative controls | **WORKS** | Workload outside the cgroup, programs on the workload cgroup: 0 reports in 150/150 boots. Hooks at root attached before init's binds: 50/50 boots reported `61000` plus `61001-62024` from pid 1, so the problem is real. |
| (c) root programs still effective | **WORKS** | Host-observed flow stream header byte-identical to the root controls in 80/80 boots; missing in 20/20 when the root sockops program is not attached. |
| (d) cost | **PARTIAL** | Programs in the existing object: +0.88 ms p50 (p90 +1.66, max +2.49). A second `Ebpf::load`: +20.3 ms p50, which meets the ≥ 20 ms falsification bar. |

Notes:

- (b) Hooks at root attached at the current end of the production program loop (after the binds) were also silent, 50/50. The cgroup's value is order-independence, not a different result for today's order.
- (c) Direct polling of `CHILD_CORRELATION` saw no entries in any arm, controls included, because the flow owner consumes them; the host-received stream header (cookie 1, destination `10.250.77.9:8080`) is the observable. A first run showed 1 to 3 headers per boot; that was a driver bug (records accumulating by reused CID), fixed and re-run.

## Cost (40 interleaved boots per variant, init start to READY; two runs agreed within 0.2 ms)

| Comparison | Paired result |
|---|---|
| Design (programs in existing object) vs same object without them | p50 +0.88 ms, p90 +1.66, max +2.49 |
| Programs and their two maps carried in the object, nothing attached | −0.01 ms |
| Workload cgroup `mkdir` + placement | p50 +0.01 ms (0.07 ms absolute) |
| Second `Ebpf::load` | +20.4 ms |

- The second-load cost is aya 0.13.1 `EbpfLoader::new()` parsing kernel BTF eagerly (`aya-0.13.1/src/bpf.rs:169-171`); `.btf(None)` does not help (−0.3 ms) because the parse already happened. Attaching the three programs takes 0.56 ms.
- The same-object arms compare against a spike-built copy of the guest object; the xtask-built object loads about 2.8 ms slower in this guest (not investigated).

## Kernel citations (Linux v7.0)

- Socket cgroup is the creating task's: `net/core/sock.c:2330` → `cgroup_sk_alloc` (`kernel/cgroup/cgroup.c:7408-7434`, `task_css_set(current)->dfl_cgrp` at `:7423-7425`); accepted children inherit via `cgroup_sk_clone` (`sock.c:2538`).
- Programs run in the socket's cgroup: sock_ops `kernel/bpf/cgroup.c:1721-1727`, post_bind `:1639-1646`, `include/linux/bpf-cgroup.h:216-235,344-353`.
- Effective-program inheritance: `compute_effective_progs`, `kernel/bpf/cgroup.c:432-490`; a parent's programs are included only if flagged `BPF_F_ALLOW_MULTI` or the child has none of that attach type (`:443-447`, `:458-484`).
- Link attach forces `BPF_F_ALLOW_MULTI`: `kernel/bpf/cgroup.c:1506-1507` (`cgroup_bpf_attach(..., BPF_F_ALLOW_MULTI | attr->link_create.flags, ...)` in the link-create path). Re-read in a v7.0 checkout on 2026-11-04; this spike's earlier `:1502-1505` was wrong (those lines are `bpf_link_prime` error handling).
- `CLONE_INTO_CGROUP`: `include/uapi/linux/sched.h:38`, `kernel/fork.c:2840-2842`, `kernel/cgroup/cgroup.c:6769-6838` (the comment at `:6824-6837` calls it the atomic form of writing the child pid to `cgroup.procs`; without the flag a forked child takes its parent's css_set, `:6791-6794`).

## Design implications

1. PR-2 holds as written. "A separate small sockops program, in the same object" is load-bearing: separate must not mean a separate object.
2. Root programs stay effective in the child only because they are link attachments. A legacy `BPF_PROG_ATTACH` `Single` without `ALLOW_MULTI` would drop the root sockops program for a child that has its own (`kernel/bpf/cgroup.c:459`). Only sock_ops has this interaction; post_bind are different attach types.
3. `clone3` replaces `Command::spawn` for the workload. Exec failure then shows only as exit status 127 where `InitError::Spawn` is returned today; a missing workload binary was not exercised. This is crafter-level structure, not an interface contract.
4. Reports cover only sockets created by tasks in the workload cgroup. A workload that moves itself out is never reported, which fails closed.

Not shown: the production reader and beacon `LISTEN` path, IPv6 and the 26 listen-hooks cases (covered by `findings-listen-hooks.md`), and a later init socket that would leak under the ordering alternative.
