# E-G3 receipt: unprepared guest kit (CA-4)
Commit 36618cba7 (dirty tree). Box kernel 7.0.0-38-generic.
- Kit path: `<checkout>/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer` (xtask `guest_peer_path`).
- 00:52:10Z `metal run --exclusive -- sh -c 'mv $P $P.aside'` run f7067bfc4cad4a660a9c22e0 (exit 2 only from the follow-up `ls` of the moved-away path; the move succeeded).
- 00:52:36Z `cargo xtask metal run --benchmark guest-flow-forwarding` run 507cd3e5a68532d2bedd7e86, exit 0, finished 00:57:53Z. Result dir benchmarks/results/guest-flow-forwarding/507cd3e5a68532d2bedd7e86 (pulled; record.jsonl, report.md).
- Record: 34 lines; 30 replicate lines `outcome:failed`, each `guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at <path>; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane`; 0 `guest_observed` lines; no artifacts/ dir (no console, no VMM launched); terminal `{"t":"run_finished","replicates_ok":0,"replicates_failed":30}`.
- Report lines 28-34: guest vcpus / memory mib / kernel / clocksource / artifact overdrive_init / overdrive_guest_peer / ovd_fwd each `absent: no replicate returned a guest view`.
- Restore: 00:58:14Z `metal run -- cargo xtask guest-kit-prepare` run 24665181d8f35f4f831002fa exit 0 ("the lane is prepared"); peer back (00:59, 864928 B). A leftover `overdrive-guest-peer.aside` copy remains in the box target dir (harmless).

## Verified against the record
Checked against benchmarks/results/guest-flow-forwarding/507cd3e5a68532d2bedd7e86/{record.jsonl,report.md}. Held: 34 lines (run_started, 2 canary, 30 replicate, run_finished); all 30 replicates `outcome:failed`, every failure starts `guest boot:` and names `cargo xtask guest-kit-prepare`; terminal `replicates_ok 0, replicates_failed 30`; no `guest_observed` line; no artifacts/ directory; report shows replicates ok 0 / failed 30 / `status: finished`, the seven guest lines (vcpus, memory mib, kernel, clocksource, three artifacts) as `absent: no replicate returned a guest view` (lines 28-34), and a Failures section with 30 entries, each naming profile, block, arm `forwarding` and the reason.
Only reported (not in the repo): the box-side moves and restore (runs f7067bfc..., 24665181...), the exit codes, and the leftover `.aside` file.
