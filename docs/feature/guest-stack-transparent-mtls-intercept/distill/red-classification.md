# RED classification — guest-stack-transparent-mtls-intercept (GH #222)

Pre-DELIVER fail-for-the-right-reason gate. Per `nw-distill` § "Pre-DELIVER
fail-for-the-right-reason gate": each scaffolded AT must be **RED for the right
reason** (implementation missing — `MISSING_FUNCTIONALITY`), never BROKEN
(import/fixture/setup error). DELIVER reads this at RED-phase entry (ADR-025 D2).

## Compile-check evidence (Lima, this wave)

- BPF object built (`cargo xtask bpf-build`, prereq for `overdrive-dataplane`
  `build.rs`).
- `cargo xtask lima run --no-sudo -- cargo check -p overdrive-control-plane
  --all-targets --features integration-tests` → **Finished, 0 errors** (the
  default-lane `VmTapPlan` scaffold + the S-GTI-09/10/11 derivation test module
  compile — imports resolve, RED not BROKEN).
- `cargo xtask lima run --no-sudo -- cargo check -p overdrive-cli --all-targets
  --features integration-tests,kvm-tests` → **Finished, 0 errors** (the
  Tier-3 metal AT file compiles under the `kvm-tests` gate).
- **2026-08-28 follow-up (S-GTI-12 added):** re-ran
  `cargo xtask lima run --no-sudo -- cargo check -p overdrive-cli --all-targets
  --features integration-tests,kvm-tests` → **Finished, 0 errors** — the new
  `a_stopped_microvm_workloads_egress_mesh_guard_is_torn_down_never_left_behind`
  scaffold (S-GTI-12, `#[should_panic(expected = "RED scaffold")]`) compiles RED,
  not BROKEN, under the `kvm-tests` gate.

## Per-scenario classification

| ID | Layer | RED reason | Classification | Runnable this wave? |
|---|---|---|---|---|
| S-GTI-01 | Tier-3 metal | Tap wire + C3 VM branch + D6 gate flip unbuilt; guest has no NIC so dial-by-name cannot resolve | `MISSING_FUNCTIONALITY` | **NO — Tier-3 metal-deferred** |
| S-GTI-02 | Tier-3 metal | Q9 EXEC-release deferral unbuilt; today `EXEC` fires inside `driver.start()` before `start_alloc` (`vm_driver.rs:917-951`) — the first-connect window is OPEN | `MISSING_FUNCTIONALITY` | **NO — Tier-3 metal-deferred** |
| S-GTI-03 | Tier-3 metal | No tap wire ⇒ no captured guest egress ⇒ no inter-agent mTLS leg to scan | `MISSING_FUNCTIONALITY` | **NO — Tier-3 metal-deferred** |
| S-GTI-04 | Tier-3 metal | Guest has no NIC ⇒ no NonMesh dial to pass through | `MISSING_FUNCTIONALITY` | **NO — Tier-3 metal-deferred** |
| S-GTI-05 | Tier-3 metal | D6 gate is `Exec`-only at `:1584`; a VM alloc never reaches `start_alloc`, so there is no install to fail-close on | `MISSING_FUNCTIONALITY` | **NO — Tier-3 metal-deferred** |
| S-GTI-06 | Tier-3 metal | D6 restart gate `:1880` is `Exec`-only; a restarted VM alloc never re-installs | `MISSING_FUNCTIONALITY` | **NO — Tier-3 metal-deferred** |
| S-GTI-07 | Tier-3 metal | C3 VM branch does not inject `workload_addr = guest_addr`; `workload describe` shows no guest addr | `MISSING_FUNCTIONALITY` | **NO — Tier-3 metal-deferred** |
| S-GTI-08 | Tier-3 metal | Guest addressing + the Q7 EXIT-before-EXEC host arm unbuilt | `MISSING_FUNCTIONALITY` | **NO — Tier-3 metal-deferred** |
| S-GTI-12 | Tier-3 metal | The VM install path (D6 gate flip `:1584`/`:1880` + tap wire + C3 branch) is unbuilt, so a VM alloc never installs the intercept — the precondition (a VM alloc whose `overdrive-mtls` rule is PRESENT) cannot be established, so there is no rule for `stop` to tear down. Once GREEN it locks the ungated-by-`DriverType` teardown (`:1269`/`:2038`): a teardown `DriverType::Exec` gate would leak the rule and red this AT (feature-delta § [REF] D6 MIRROR hazard) | `MISSING_FUNCTIONALITY` | **NO — Tier-3 metal-deferred** |
| S-GTI-09 | layer-1 unit | `derive_vm_tap_plan` is a `todo!()` RED scaffold | `MISSING_FUNCTIONALITY` | YES (default lane / Lima) — `#[should_panic]` catches the todo panic |
| S-GTI-10 | layer-1 unit | `derive_vm_tap_plan` guest-carve unbuilt (`todo!()`) | `MISSING_FUNCTIONALITY` | YES (default lane / Lima) |
| S-GTI-11 | layer-1 unit | `derive_vm_tap_plan` MAC derivation unbuilt (`todo!()`) | `MISSING_FUNCTIONALITY` | YES (default lane / Lima) |

**Zero `IMPORT_ERROR` / `FIXTURE_BROKEN` / `SETUP_FAILURE` / `WRONG_ASSERTION`
classifications.** All twelve ATs are RED for the correct reason (implementation
missing). No scenario blocks handoff. (S-GTI-12 added in the 2026-08-28
follow-up — the teardown-ungated regression lock closing Sentinel finding F1.)

## Why the nine metal ATs are metal-deferred, not fakeable

The spike's load-bearing constraint (`../spike/wave-decisions.md` § "Constraints
Discovered"): a faithful test REQUIRES a real guest kernel behind a real
virtio-net tap — **a netns cannot model "no host `struct sock`"** (its TCP
terminates in the HOST kernel). Substituting a Lima/sim/netns test that skips a
real guest boot would test a DIFFERENT thing and mask the exact interception
mechanism #222 exists to prove. Nested KVM (`kvm-tests` via `cargo xtask metal
run --`, the x86_64 metal box) is the ONLY honest execution surface — arm64 Lima
cannot boot the guest. This wave has no metal box, so the nine bodies (S-GTI-01..08
+ the S-GTI-12 teardown lock) are scaffolded RED and their execution is deferred
to DELIVER on the metal box (S-GTI-12 needs a real VM alloc that actually installs
the `overdrive-mtls` rule — the same nested-KVM surface). This
is an HONEST RED (compiled, discoverable, `#[should_panic(expected = "RED
scaffold")]`), never a faked GREEN.

## DELIVER GREEN-phase transition (per scaffold)

- **S-GTI-09/10/11 (layer-1):** implement `derive_vm_tap_plan` (remove the
  `todo!()`), then drop each `#[should_panic]` + `panic!` line, run the already-
  documented assertions, and convert each to a `proptest!` over the `NetSlot`
  domain (`@property`, Mandate 9 — C1 boundary slots 0 / `NET_SLOT_MAX`, C3).
  Add the symmetric guest-carve const guard beside the S6 transit guard
  (`veth_provisioner.rs:518`) — the compile-time companion S-GTI-10 motivates
  (Q6 / Finding 4).
- **S-GTI-01..08 + S-GTI-12 (Tier-3 metal):** replace each `#[should_panic]` +
  `panic!` with a `#[tokio::test] #[serial(cgroup)]` body driving
  `overdrive_cli::commands::{deploy,workload::describe,deploy::stop}` (the
  `vm_walking_skeleton.rs` shape) against a real mTLS-composed `serve` on the
  metal box. The module is already in the `host-kernel-shared` nextest group.
  Observe the east-west corollary (the dialer speaks PLAINTEXT; prove mTLS on the
  inter-agent wire). **S-GTI-12** specifically: after `deploy` reaches Running,
  assert the VM alloc's `overdrive-mtls` rule is PRESENT (host `nft list`), drive
  `deploy::stop`, then assert that rule is GONE and no other alloc's rule moved —
  and treat the teardown-ungated invariant as locked (a `DriverType::Exec` gate on
  `:1269`/`:2038` must red this AT). See § "DELIVER carry-forwards" in the
  feature-delta § Wave: DISTILL for the F2–F5 assertion-strength obligations that
  ride alongside these bodies (S-GTI-06 restart primary-lock, S-GTI-08
  restart_count, C6a malformed-token sad path, metal `#[ignore]`-vs-`#[should_panic]`).
</content>
