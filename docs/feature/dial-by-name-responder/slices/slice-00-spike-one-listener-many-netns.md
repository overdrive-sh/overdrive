# Slice 00 — SPIKE: one listener, many netns

> Reviewed brief (DISCUSS, 2026-06-24; gated to Slice 00). Feature: `dial-by-name-responder` (#243). Story: **US-DBN-1**
> (`@infrastructure` `@spike`). Job: J-MESH-001. **BLOCKING — runs
> before the walking skeleton.**

## Goal (one line)

Validate that ONE host-side in-agent listener can receive and answer DNS queries
sent to N **different** per-netns gateway addresses, on a real kernel — before any
production code is designed.

## Learning hypothesis

A single listener bound on the host side can serve every per-workload netns'
`resolv.conf` gateway address (the D-TME-9 injection target). **Predicted:** WORKS
(the gateway addr is the host-side veth peer; the host listener receives queries
routed out of each netns). **Falsification:** queries from inside netns B never reach
the listener (routing/binding requires per-netns sockets), forcing a design pivot
(e.g. per-netns listener, or a different bind shape) before the skeleton.

## What to probe (NOT design — `spike.md`)

- In gitignored `spike-scratch/increment-a/` (self-contained Cargo project, NEVER `crates/`).
- Provision ≥2 per-workload netns+veth reusing the shipped `veth_provisioner` topology shape (per-netns gateway = `plan.host_addr`).
- Inject each netns' `resolv.conf` → its own gateway addr.
- Run ONE host-side listener; from inside each netns emit a real `getaddrinfo`/`dig`-shape query toward that netns' gateway.
- Confirm the one listener receives and answers all of them; record the bind/routing shape that worked.

## Carpaccio taste tests

- **Closes a real loop?** It's a PROBE, not a ship — produces a verdict, not operator value (the `@infrastructure` exemption). It de-risks the production loop the next slice closes.
- **Thinnest?** Yes — 2 netns, 1 listener, the single routing question.
- **No `#[test]`-only composition?** N/A — runs for real under Lima as root, not a test.

## Acceptance (probe gate)

- [ ] Runs for real under Lima as root (`cargo xtask lima run -- …`); NO `--no-run`/compile-only.
- [ ] `spike/findings.md`: binary verdict (WORKS/DOESN'T-WORK), **pasted** output, `uname -r`, the working bind/routing shape (or the wall).
- [ ] `spike/wave-decisions.md`: PROMOTE / DISCARD / PIVOT.
- [ ] Code in `spike-scratch/` only; eBPF (if any) is aya-rs Rust, never C.

## Dependencies / notes

- No Tier-2 backstop (`spike.md` "no synthetic harness" case) — only a real kernel signal counts.
- Output gates Slice 01. If DOESN'T-WORK, the headless-in-agent design pivots here, cheaply.
