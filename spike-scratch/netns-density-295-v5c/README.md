# netns-density-295 — V-5(c) spike (guest UDP re-association; guest cell residue, K-B3)

Throwaway. Findings: `docs/feature/netns-density-295/spike/findings-v5c.md`.

Derived from `spike-scratch/netns-density-295-v11-vip-v14/` (the guest UDP slot mechanism: slot UDP socket with a
stream verdict, TCP framing cell p1->p2, TCP reassembly cell p3->p4 with strparser, `connect4`/`sendmsg4`
capture, guest-`lo` TC unframe). Additions: `--cells none|empty|recreate`, `--order old|cutfirst`,
host fault injection (`--refuse-dst`, `--abort-dst`, `--partial-dst`, `--fault-odd`, `--refuse-with`,
`--refuse-delay-ms`), guest `--pair-deadline-ms` and `--cell-bufs`, tokenised epoll registrations,
private vsock ports (45234-45236, 45000), recording UDP destinations in a private netns, no use of the shared
`lo`. Guest CID 3261; every resource is prefixed `v5c`.

- `bpf/` Aya eBPF ELF (unchanged from the V-11 spike). `vfwd/` owners and test applications.
- `harness.py`, `v5c.py` host harness and scenario library. `launch.py` runs an increment on the metal host
  WITHOUT `cargo xtask metal run` (copies only this directory, own flock, own cleanup).
- `increment-a` smoke. `increment-b` invalidated run (shared-`lo` TCX interference, fd-reuse spurious
  associations), kept as evidence. `increment-c` clean full suite (old teardown order). `increment-d`
  interrupted at the 30-minute tool limit (A1, B4 follow-up, a B5 with a wrong oracle). `increment-e`
  remainder of the suite and the revised B5. Each `evidence/` holds raw logs, per-scenario owner logs,
  destination logs, `results.json`. `increment-b/PREREGISTRATION.md`, `increment-d/PREREGISTRATION.md`,
  `increment-e/PREREGISTRATION.md` were written before the respective runs.
