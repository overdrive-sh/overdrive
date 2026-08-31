# Immutable-baseline RED classification — bounded lifecycle/network correction

**Classification base:** `465b96c39083984a1d2d470caff918a723b9301f`

**Feature:** `guest-stack-transparent-mtls-intercept`

**Scope:** BTR-1, BTR-2, and BTR-3 only.

The baseline was inspected from the committed production tree before the
DISTILL scaffolds were added. Each implementation gap is reachable through the
current production `action_shim::dispatch` path. The scaffolds are deliberately
`#[should_panic(expected = "RED scaffold")]`: a targeted runner pass confirms
that the Rust test harness, imports, and registration are sound, but is not a
GREEN behavior claim. DELIVER removes the marker while activating the real
port-to-port body.

| Scenario | Immutable production evidence | Scaffold classification | Fail-for-right-reason |
|---|---|---|---|
| S-GTI-BTR-01 | `StopAllocation` uses an unbounded `loop`; every `Ok(None)` returns to another fresh read/proposal (`action_shim/mod.rs:2615-2695`). Route removal is conditional on an accepted occurrence (`:2709-2712`), contrary to the exhausted/exact-terminal tails. | `RED_SCAFFOLD_PRESENT` | `MISSING_FUNCTIONALITY`: no two-proposal bound or unconditional no-event route tail exists. |
| S-GTI-BTR-02 | `provision_and_inject_netns` assigns at `:1129` and returns a provision error at `:1146`; both action failure paths can record Failed without first using `teardown_and_release_netns_raw` (`:1762-1783`, `:2183-2227`). | `RED_SCAFFOLD_PRESENT` | `MISSING_FUNCTIONALITY`: the post-assignment unwind and cleanup/store precedence are absent. |
| S-GTI-BTR-03 | Same-id restart awaits prior driver stops (`:2136-2158`) and then enters replacement provisioning directly (`:2168-2188`). Prior mTLS/structural cleanup currently occurs only in later abort handling through the superseded `RestartNetworkDisposition` protocol (`:1345-1370`, `:2192-2198`). | `RED_SCAFFOLD_PRESENT` | `MISSING_FUNCTIONALITY`: prior protection can still exist when replacement provisioning begins. |

## Targeted execution

The canonical command and result are recorded after scaffold creation:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-control-plane \
  --test acceptance -E 'test(/(stop_allocation_second_lww_rejection_completes_without_event|post_assignment_provision_failure_tears_down_before_slot_release|same_id_restart_removes_prior_protection_before_replacement_provision)/)'
```

The expected DISTILL result is three runner passes whose reason is the exact
expected `RED scaffold` panic. Any compile, import, fixture, timeout, or
unexpected-panic failure is `BROKEN` and blocks handoff. Actual output is added
below after execution.

**Result:** PASS — nextest run
`d3823317-38f3-4b90-b6ea-0ebc55196dd2` executed 3 tests; all 3 passed by
matching their intentional `RED scaffold` panic and 338 unrelated tests were
filtered out. The prerequisite `cargo xtask bpf-build` completed first because
the production dataplane embeds the generated BPF object at compile time.

## Superseded-test transition

The committed integration test
`restart_provision_failure_cleans_prior_intercept_without_releasing_the_slot`
asserts the rejected retain-for-retry behavior. It is not evidence for the new
contract and must be transitioned with BTR-2/BTR-3 implementation. Other
restart-abort tests that place prior mTLS/network cleanup after replacement
provisioning must be updated to the prior-protection-first trace. This is
bounded test fallout, not permission to retain or redesign
`RestartNetworkDisposition`.

## Scope audit

- No production file or public/test-only surface was changed.
- No expectation, example, shell/Python harness, roadmap, or review artifact
  was changed.
- No cancellation, replay, route-hydration, broker/relist, liveness-attempt,
  probe-schema/signature, expanded boot-GC, receipt/outbox, or retry-owner
  scenario was added.
- The pre-existing dirty `AGENTS.md` is excluded from this classification and
  from the eventual commit.
