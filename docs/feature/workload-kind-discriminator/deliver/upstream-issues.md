# Upstream sequencing notes — workload-kind-discriminator

Notes from per-step DELIVER execution where the slice-by-slice
ordering surfaces a load-bearing dependency that the step in question
cannot land in isolation. Each note names the affected slice, the
file that must change, and the slice that lands the change so the
sequencing becomes a dependency, not a deferral.

These are NOT GitHub-issue deferrals — they are sequencing artefacts
between sibling slices in this feature, all of which land before the
feature is finalised. Per CLAUDE.md "Deferrals require GitHub issues",
nothing here is left as a hand-wavy forward pointer; each note
identifies the specific slice that closes the loop.

## Slice 05 (step 01-02) — Schedule submit-handler integration

**Status**: blocked-by-sibling-slice. Lands in slice 02 (step 02-01).

**Surface**: `crates/overdrive-cli/src/commands/job.rs::submit_streaming`
currently parses TOML via the legacy `JobSpecInput` shape with
`deny_unknown_fields`. To wire Schedule end-to-end through the CLI
submit handler, `submit_streaming` must:

1. Parse via `WorkloadSpecInput::from_toml_str`.
2. Branch on the `WorkloadKind` discriminator and dispatch to the
   per-kind streaming sub-path: Service → existing convergence loop;
   Job → existing convergence loop; Schedule → emit
   `ScheduleSubmitEvent::Accepted` + `ScheduleSubmitEvent::Registered`,
   close the stream.
3. Render via the kind-aware dispatcher (`render::schedule::*` for
   Schedule, the existing `format_running_summary` /
   `format_failed_block` for Service/Job).

This is exactly the slice 02 surface ("Job submit terminal verdict")
extended to the third kind. Slice 05 (this step) lands the
prerequisite render functions, the SSOT deferral URL constant, the
`ScheduleSubmitEvent` sibling enum, the canonical
`IntentKey::for_schedule` derivation, and the IntentStore persistence
contract — every artefact slice 02's discriminator wiring will route
to. The slice 02 step (when it lands) will:

- Consume `ScheduleSubmitEvent` instead of inventing a fourth event
  type.
- Read `SCHEDULE_EXECUTION_TRACKING_URL` from the SSOT in the CLI
  consumer side as well, completing the KPI K5 byte-equality property
  end-to-end.
- Use `IntentKey::for_schedule` for the Schedule persistence path
  (currently slice 05 ships only the helper; slice 02's handler call
  site will use it).

**No GitHub issue required**: this is intra-feature sequencing, not a
deferral. Both slices land in the same feature before the
workload-kind-discriminator artifacts are finalised.

**Acceptance evidence stays valid**: the slice 05 scenarios
(`crates/overdrive-cli/tests/integration/job_submit_schedule.rs`)
exercise the render functions, the constant, and the IntentStore
helper directly — port-to-port at the appropriate scope. KPI K5
byte-equality is asserted at the render-layer boundary in slice 05;
the slice 02 wiring will extend the assertion through the CLI
streaming consumer surface as well.
