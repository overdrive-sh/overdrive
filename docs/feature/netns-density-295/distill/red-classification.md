# Pre-DELIVER RED classification — `netns-density-295`

The repository is greenfield. DISTILL owns the complete acceptance bodies;
DELIVER performs the accepted single-cut API replacement and activates the
reasoned-pending tests in roadmap order. No compatibility API or migration
machinery is required.

## Current executable results

| Surface | Result | Classification |
|---|---|---|
| Existing live workspace suite | GREEN | Existing behavior remains protected before DELIVER starts the cut. |
| `SimSharedGuestNetworkOwner` adapter tests | GREEN | Deterministic port-output scripting and ordered call observation are test infrastructure, not feature implementation. |
| Core EXEC-gate acceptance (`crates/overdrive-core/tests/acceptance/netns_density_exec_gate.rs`) | Fails at the accepted gate scaffold | **RED — MISSING_FUNCTIONALITY**. |
| Guest-network provisioner acceptance (`crates/overdrive-control-plane/tests/acceptance/netns_density_guest_network.rs`) | Fails at the accepted guest-network scaffold | **RED — MISSING_FUNCTIONALITY**. |
| Worker EXEC-release acceptance (`crates/overdrive-worker/tests/acceptance/netns_density_exec_release.rs`) | Authored in its live home, `mod`-line commented pending F-01 — the body targets the post-F-01 seven-argument `VmDriver::new(…, wiring.gate(), layout)` and gate-coupled `release_for_exit_emission` (recovery waits, FailStop wakes without writing EXEC), which the pre-cut tree does not yet provide, and wiring it today would be a compile break rather than a reasoned-`#[ignore]` RED | **RED — MISSING_FUNCTIONALITY** at activation, once DELIVER's F-01 step lands the seven-arg ctor and un-comments the `mod` line in the same cut. |

No active test is classified as BROKEN. Tests that require the final grouped
`AllocationSpec.network` shape remain complete and reasoned-pending until the
DELIVER step that performs that single cut. The crafter may activate them but
may not author, replace, weaken, or repair their bodies.

## Completeness disposition

The prose specification covers all 15 applicable completeness checks. The
acceptance bodies and their target homes are authored. Final-shape bodies that
cannot compile against the deliberately pre-cut public type remain pending the
DELIVER API-cut step; this is a staging fact, not a compatibility requirement
or authorization for the crafter to redesign the tests.
