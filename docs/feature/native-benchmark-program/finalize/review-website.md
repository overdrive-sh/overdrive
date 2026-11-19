# Website documentation and blog review — native benchmark program

Reviewer: independent documentation reviewer (nw-documentarist-reviewer, read-only), 2026-11-19.

## Round 1 — CHANGES_REQUESTED

MAJOR
1. Blog: "about 67 ns per program run. That is 0.48% of a loopback connect" attaches 0.48% to the wrong quantity. The spike (findings-bpf-stats-overhead.md:11,56) gives +145 ns across two connect programs on a 30.4 µs connect = 0.48%; 67 ns is 0.22%.
2. benchmark-methodology.mdx:12 "No state survives from one replicate to the next" overclaims: the fixture is fresh but the host page cache persists (the vm-lifecycle cold-cache profile drops it per replicate); contradicts line 58.

MINOR
3. "the one case"/"the one measurement" taking exclusive admission: expectation captures and maintenance also run --exclusive.
4. "a dedicated form of the Metal runner" → name the command.
5. Future-hint wording ("cannot be benchmarked yet", "The only environment today", "is also being replaced") → present facts.
6. Undefined "spikes", "canary steady", "Mann-Whitney p-value" at first use; upper-bound claim on relative overhead needs its source.
7. Bare "#361"/"#362"/"#303" link text → descriptive link text.
8. Blog description "none of their numbers were comparable" contradicts the body.
9. "the host closes the console channel" → the beacon connection is closed (findings-v10-guest-boot.md:43).

Checks that passed: every other blog number matches the four current reports and the spike records; no in-noise delta presented as a finding; decision story complete; no process IDs or phase names; commands, flags, exit codes and diagnostics match code and rules; navigation and frontmatter correct.

## Round 2 — APPROVED

All nine round-1 findings resolved; corrected numbers verified against findings-bpf-stats-overhead.md, findings-v1-bpf-stats-fd.md and findings-v10-guest-boot.md; the upper-bound reason matches the DDD-13 paragraph; mTLS egress figures re-checked against mtls-path/5bf66f2f594d8d8c1bd5676e; no new defects. Validation: bun typecheck, lint, build (49 pages) and assert:one-index pass; test:blog not run (it requires the opennextjs/wrangler Worker build).
