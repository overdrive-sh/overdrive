# Review — finalization content (guest-benchmark-substrate), 2026-11-20

Independent reviewer: `nw-documentarist-reviewer`. Scope: the how-to and concepts pages, the new blog post, the corrected sentence in the earlier benchmark post, and the evolution document, checked against the feature delta, ADRs 0325-0333, the spike findings, the evidence receipts, the catalogue, the methodology and the retained reports.

## Round 1 — CHANGES_REQUESTED

Major: (1) the blog said the family ran once; a second clean run (`62cf21ac0f4b8793e31ec89d`, p50 40.0 µs against 50.5 µs) was unreported; (2) cost figures did not say what they measured (scripted-peer spike against the delivered 69 s run; 83/84 s were estimates); (3) the guest kit and `Replicate` were used without introduction; (4) the evolution document lacked the finalization record. Minor: vsock/console ratio wording, the pooled p50's place relative to the high cluster, attribution of the leading-newline result to the spike, box-side probes stated as reported, the `guest_observed` wording, and the how-to prerequisites (verified: `guest-kit-prepare` runs `bpf-build` and `kmod-build`). The orchestrator also corrected the how-to's console-grammar sentence to the printer in `guest_app.rs` (no count, SHA-256 checksum).

## Round 2 — APPROVED

Every in-scope finding resolved. Four low nits: two applied (the guest kit defined on the how-to page; the newline written before the program's first line, not before the block); two not applied (a CID-count phrasing in the blog, glosses for "spike", "scripted test peer", "record sink" and "no-beacon timeout").
