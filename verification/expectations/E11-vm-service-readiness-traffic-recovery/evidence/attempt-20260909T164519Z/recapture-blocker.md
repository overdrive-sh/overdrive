# E11 recapture disposition — Stable observation surface is unavailable

## Capture result

The fresh native-metal capture at `2026-09-09T16:45:19Z` succeeded through
the existing E11 runner and built default-feature product. Its complete
receipt, metadata, transcript, ledger, runner log, dirty-state capture, and
provenance are retained in this directory and in
`evidence/attempt-20260909T164519Z/`.

The fresh ledger and transcript directly show the other predicates:

- readiness `Pass -> Fail -> Pass`, with transition latencies `338 ms`,
  `195 ms`, and `160 ms` (`readiness-recovery.tsv:2-4`);
- the same allocation `alloc-service-vm-readiness-recovery-0` stayed
  `Running` with `0` restarts in all three public descriptions
  (`product-run.out:36-38,72-74,108-110`) and ledger rows;
- peer results are exact reply, unreachable/no exact reply, and exact reply
  (`readiness-recovery.tsv:2-4`); and
- teardown ended with the complete zero-delta line
  (`product-run.out:148-151`).

The transcript records `Stable` only for the initial submit stream at
`product-run.out:22-32`. The post-withdrawal and post-recovery public
descriptions at `product-run.out:69-88` and `105-124` contain `Running`, the
probe observations, and no `Stable` observation. The extracted ledger at
`readiness-recovery.tsv:1-4` has no Stable field. This is the same missing
evidence identified by the independent audit; the fresh run does not change
that fact.

## Exact surface evidence

The existing operator surface cannot expose a post-transition Stable
observation without a production CLI/API change:

1. `crates/overdrive-cli/src/cli.rs:101-108` defines `workload describe` as
   `Describe { id }` with no additional output mode or terminal-observation
   surface.
2. `crates/overdrive-cli/src/render.rs:1095-1144` renders the Service body as
   `Alloc / State / Restarts / Since`, followed by cause and last-terminated
   details. It does not render the current `TerminalCondition::Stable`.
3. `crates/overdrive-control-plane/src/api.rs:377-424` defines
   `AllocStatusRowBody`; it carries state, reason, transition, error, restart,
   and prior `last_terminated` data, but no current terminal field.
4. `crates/overdrive-control-plane/src/handlers.rs:1219-1257` projects the
   current rows into that body, so the omitted Stable claim is not available
   on the HTTP snapshot consumed by `workload describe`.
5. `crates/overdrive-control-plane/src/streaming.rs:886-985` makes the Service
   submit stream emit `Accepted` and one terminal event for that submit, then
   closes. It has no post-submit snapshot replay that could expose the
   already-announced Stable claim after either readiness transition.

The existing E11 example and runner therefore have no truthful, approved
operator observation from which to populate post-withdrawal or post-recovery
`Stable` fields. Adding a CLI/API field, a renderer branch, a new stream
replay, or a derived ledger label would invent or alter public surface and
would not be an evidence-only remediation. The current artifact deliberately
does not do that.

## Blocker and next boundary

This attempt is **BLOCKED — NEEDS_RECAPTURE remains unresolved** for the full
roadmap contract. The native capture is retained as executed partial evidence;
it is not marked `satisfied`, and no README, INDEX, prior receipt/transcript,
or DES history is edited. Closing E11 requires an approved existing/public
surface that directly emits Stable after withdrawal and recovery, or an
explicitly approved design/API change followed by its own implementation and
review workflow. No such change is authorized by this remediation.
