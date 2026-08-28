//! Tier-3 metal ATs for guest-stack transparent-mTLS intercept — EGRESS (GH #222).
//!
//! DISTILL-authored RED scaffolds (nw-distill Mandate 7 / ADR-025: DISTILL is the
//! canonical AT author; DELIVER unskips + implements the bodies AND the production
//! wiring). Companion spec: `docs/feature/guest-stack-transparent-mtls-intercept/
//! distill/test-scenarios.md` (S-GTI-01..08 + S-GTI-12) and the RED classification
//! at `.../distill/red-classification.md`.
//!
//! # Why these are `#[should_panic(expected = "RED scaffold")]`, metal-deferred
//!
//! Every scenario here boots a REAL Cloud-Hypervisor microVM — a netns CANNOT
//! model "no host `struct sock`" (the increment-n spike's whole point), so a
//! faithful test needs a real guest kernel behind a real virtio-net tap. That
//! requires **nested KVM**: the file is gated behind
//! `#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]` and runs
//! under `cargo xtask metal run --` on the `x86_64` metal box, NEVER the Lima inner
//! loop (arm64 Lima has no nested KVM; a Lima run returns no signal because the
//! guest never boots). This DISTILL wave has no metal box, so the nine bodies are
//! scaffolded RED per `.claude/rules/testing.md` § "RED scaffolds" and the
//! fail-for-right-reason classification records them **Tier-3 metal-deferred**.
//! DELIVER GREEN-phase replaces each `panic!` + `#[should_panic]` with the real
//! `#[tokio::test] #[serial(cgroup)]` body driving `overdrive_cli::commands::
//! {deploy,workload::describe,deploy::stop}` (the shape `vm_walking_skeleton.rs`
//! already proves out), and joins this module to the `host-kernel-shared`
//! nextest group (`.config/nextest.toml`) — every scenario boots a full
//! production `run_server` against the shared real host cgroupfs.
//!
//! # The production call sites these ATs drive (NO test-only wiring — CLAUDE.md
//! # vertical-slice bar; the #236 dead-mechanism precedent is the counter-example)
//!
//! Each scenario drives ONLY production entry points — `overdrive serve`
//! (mTLS-composed boot) + `overdrive deploy vm-job.toml` + `overdrive workload
//! {describe,restart,stop}` — never a hand-installed rule/route/address. The NEW
//! production wiring DELIVER must land for these to go GREEN (feature-delta
//! § Component decomposition): the C3-seam VM branch (tap converge + guest-net
//! spec injection), the `VmConfig` net-attach + `ip netns exec` + `--net tap=`,
//! guest addressing via `overdrive-init`, and the D6 intercept-gate flip at BOTH
//! `action_shim/mod.rs` install sites (`:1584` fresh-start + `:1880` restart).
//!
//! # East-west mTLS corollary — the DIALER speaks PLAINTEXT (CLAUDE.md, an RCA'd trap)
//!
//! In S-GTI-01/02/03 the guest (and any in-test dialer standing in for it) MUST
//! speak PLAINTEXT over an ordinary `TcpStream` with a byte-distinct
//! REQUEST/RESPONSE litmus. The mTLS is proven on the INTER-AGENT (leg-B ↔ leg-C)
//! wire — `0x17` TLS 1.3 `application_data` records with zero cleartext — NOT on
//! the client handshake. The egress capture lands on the agent's PLAINTEXT
//! workload-facing leg-F; a rustls dial toward the peer opens a second, peerless
//! TLS session leg-F never terminates → the handshake stalls → RST. DELIVER MUST
//! NOT copy the inbound keystone's "client presents TLS" dial shape here (RCA:
//! `docs/analysis/root-cause-analysis-dial-by-name-agent-originated-mtls-stall.md`).

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]

/// S-GTI-01 (`@walking_skeleton @driving_port @real-io @kvm`,
/// contract-shape: bounded-change) — A microVM workload dials a mesh peer by
/// name and receives the reply.
///
/// GIVEN an mTLS-composed `overdrive serve` (DNS responder up) and a mesh
/// `[service]` peer on the node reachable by name; WHEN the operator deploys a
/// `[vm]`+`[job]` whose guest dials that peer BY NAME and sends a byte-distinct
/// plaintext REQUEST; THEN the guest receives the peer's byte-distinct plaintext
/// RESPONSE, dial-by-name resolved over the routed hops (guest resolv.conf →
/// responder → resolve → dial — the FIRST thing to exercise, topology-reasoned
/// NOT spike-proven, Finding 5), and the allocation reaches Running.
#[test]
#[should_panic(expected = "RED scaffold")]
fn microvm_dials_a_mesh_peer_by_name_and_receives_the_reply() {
    panic!(
        "Not yet implemented -- RED scaffold (S-GTI-01 / microVM dials a mesh peer by name; \
         Tier-3 metal-deferred, DELIVER drives serve+deploy)"
    );
}

/// S-GTI-02 (`@walking_skeleton @driving_port @real-io @kvm @property`,
/// contract-shape: unbounded-preservation) — The guest's very first mesh dial is
/// born intercepted; no cleartext escapes.
///
/// GIVEN an mTLS-composed `overdrive serve`; WHEN the operator deploys a
/// `[vm]`+`[job]` whose guest command's FIRST action is a mesh dial by name;
/// THEN the guest's first connection is captured by the egress intercept, ZERO
/// cleartext SYN for the mesh destination ever leaves for the peer before the
/// rule is live, and the ordering invariant `install-success ≺ EXEC-release`
/// held (Finding 2 / Q9 — the deferred EXEC reply on the guest-initiated beacon
/// connection). Example-pinned at layer 3+ (Mandate 11), never PBT-generated.
#[test]
#[should_panic(expected = "RED scaffold")]
fn the_guests_first_mesh_dial_is_born_intercepted_no_cleartext_escapes() {
    panic!(
        "Not yet implemented -- RED scaffold (S-GTI-02 / born-captured first-connect safety; \
         Tier-3 metal-deferred)"
    );
}

/// S-GTI-03 (`@real-io @kvm @wire-assertion`, contract-shape:
/// unbounded-preservation) — The guest's mesh traffic travels the peer wire as
/// mTLS, never in the clear.
///
/// GIVEN a guest dialing a mesh peer through the composed egress intercept; WHEN
/// the connection carries the request and reply; THEN the inter-agent
/// (leg-B ↔ leg-C) wire carries TLS 1.3 `application_data` (`0x17`) with ZERO
/// cleartext of the plaintext litmus, and the kTLS legs report kernel-TLS
/// installed (`ss -K`). Observed on the PEER wire, NOT the plaintext dialer
/// (east-west corollary — see module doc).
#[test]
#[should_panic(expected = "RED scaffold")]
fn the_guests_mesh_traffic_travels_the_peer_wire_as_mtls_never_in_the_clear() {
    panic!(
        "Not yet implemented -- RED scaffold (S-GTI-03 / TLS 1.3 records, zero cleartext on the \
         inter-agent wire; Tier-3 metal-deferred)"
    );
}

/// S-GTI-04 (`@real-io @kvm`, contract-shape: bounded-change) — The same guest
/// reaches a non-mesh destination in the clear.
///
/// GIVEN a guest whose mesh dials are intercepted; WHEN the guest dials a
/// NON-mesh destination (outside the mesh-membership block); THEN the connection
/// passes through in the clear (`MtlsResolve` `NonMesh`) and the guest receives the
/// reply unchanged.
#[test]
#[should_panic(expected = "RED scaffold")]
fn the_same_guest_reaches_a_non_mesh_destination_in_the_clear() {
    panic!(
        "Not yet implemented -- RED scaffold (S-GTI-04 / NonMesh passthrough; \
         Tier-3 metal-deferred)"
    );
}

/// S-GTI-05 (`@real-io @kvm @error`, contract-shape: bounded-change) — When the
/// mesh guard cannot be installed, the workload is refused, never run in the
/// clear.
///
/// GIVEN an mTLS-composed `overdrive serve` where the egress intercept install
/// will fail for a VM alloc; WHEN the operator deploys a `[vm]`+`[job]`; THEN the
/// allocation is driven terminal Failed (fail-closed, D-MTLS-18 extended to VM
/// kind), the guest never runs the operator's command (EXEC-release never
/// fired), and no cleartext egress ever left the guest.
#[test]
#[should_panic(expected = "RED scaffold")]
fn when_the_mesh_guard_cannot_be_installed_the_workload_is_refused() {
    panic!(
        "Not yet implemented -- RED scaffold (S-GTI-05 / fail-closed intercept-install failure; \
         Tier-3 metal-deferred)"
    );
}

/// S-GTI-06 (`@real-io @kvm @error @restart`, contract-shape: bounded-change) —
/// A restarted microVM workload is re-enrolled in the mesh before it runs again.
///
/// GIVEN a Running mesh `[vm]`+`[job]` whose egress intercept is installed; WHEN
/// it is restarted (crash-recovery / restart budget / `overdrive workload
/// restart`); THEN the restarted allocation re-installs the egress intercept (the
/// `action_shim/mod.rs:1880` restart gate fired for VM kind), a failing re-install
/// is driven terminal fail-closed, and a restarted VM alloc NEVER runs cleartext
/// fail-open. This is the regression lock for the HIGH: a fresh-deploy-only AT
/// (S-GTI-01) goes green over a restart fail-open hole (Finding 1 / ADR-0089 §1).
#[test]
#[should_panic(expected = "RED scaffold")]
fn a_restarted_microvm_workload_is_re_enrolled_in_the_mesh_before_it_runs_again() {
    panic!(
        "Not yet implemented -- RED scaffold (S-GTI-06 / restart re-install + fail-closed, \
         :1880 gate; Tier-3 metal-deferred)"
    );
}

/// S-GTI-07 (`@real-io @kvm`, contract-shape: bounded-change) — The operator sees
/// the microVM workload's own mesh address, not its transit hop.
///
/// GIVEN an mTLS-composed `overdrive serve`; WHEN the operator deploys a
/// `[vm]`+`[job]` and runs `overdrive workload describe`; THEN the workload's
/// canonical address is the guest address (the upper-block guest /30 host, D2a),
/// NOT the transit /30 address (which carries no workload endpoint).
#[test]
#[should_panic(expected = "RED scaffold")]
fn the_operator_sees_the_microvm_workloads_own_mesh_address_not_its_transit_hop() {
    panic!(
        "Not yet implemented -- RED scaffold (S-GTI-07 / workload_addr = guest addr, D2a; \
         Tier-3 metal-deferred)"
    );
}

/// S-GTI-08 (`@real-io @kvm @error`, contract-shape: bounded-change) — A microVM
/// that cannot address its network is refused as a boot failure, not retried
/// forever.
///
/// GIVEN an mTLS-composed `overdrive serve` where the guest's net-apply will fail
/// BEFORE exec; WHEN the operator deploys a `[vm]`+`[job]`; THEN the allocation is
/// driven terminal classified as a provision/boot failure, is NOT misattributed
/// as a crashed operator command, is NOT restart-looped (the restart budget is
/// not consumed), and the guest never ran the operator's command. Q7: the host
/// distinguishes a pre-exec `EXIT` structurally (EXIT-before-EXEC, no new beacon
/// PL field), reachable because EXEC-release is deferred (Q9).
#[test]
#[should_panic(expected = "RED scaffold")]
fn a_microvm_that_cannot_address_its_network_is_refused_as_a_boot_failure() {
    panic!(
        "Not yet implemented -- RED scaffold (S-GTI-08 / net-apply fail = boot failure, \
         not restart-looped, Q7; Tier-3 metal-deferred)"
    );
}

/// S-GTI-12 (`@real-io @kvm @teardown`, contract-shape: bounded-change) — A
/// stopped microVM workload's egress mesh guard is torn down, never left behind.
///
/// GIVEN a Running mesh `[vm]`+`[job]` deployed via real `overdrive serve` +
/// `overdrive deploy`, WITH its egress intercept installed (the VM alloc's
/// `overdrive-mtls` nft rule is PRESENT — asserted as a precondition observable);
/// WHEN the operator stops it via the real stop driving port (`overdrive workload
/// stop` → `commands::deploy::stop`); THEN the VM alloc's `overdrive-mtls` nft
/// rule is GONE (teardown fired for VM kind at the ungated-by-`DriverType` stop
/// site `action_shim/mod.rs:2038`), AND no OTHER alloc's intercept rule is
/// disturbed — observed via a host `nft list` of the `overdrive-mtls` table (the
/// rule set), NOT by instrumenting the teardown line.
///
/// This is the regression lock for feature-delta § [REF] D6's MIRROR hazard: the
/// two teardown sites (`:1269` `FinalizeFailed` `!is_stable` + `:2038`
/// `StopAllocation`) are gated ONLY on `mtls_worker.is_some()`, NOT `DriverType`.
/// Adding a `DriverType::Exec` gate to either — the second, distinct bug the
/// review anticipated — would LEAK the VM alloc's nft rule on stop and stay green
/// against S-GTI-01..08 (which assert install / egress / first-connect / wire,
/// never teardown-removal). Once GREEN, this AT goes RED the moment such a gate is
/// added, pinning the teardown-ungated invariant (its install-side mirror is
/// S-GTI-06's `:1880` lock; this is the teardown twin). Example-based (Mandate 11),
/// Tier-3 metal-deferred like S-GTI-01..08.
#[test]
#[should_panic(expected = "RED scaffold")]
fn a_stopped_microvm_workloads_egress_mesh_guard_is_torn_down_never_left_behind() {
    panic!(
        "Not yet implemented -- RED scaffold (S-GTI-12 / stop tears down the VM alloc's \
         overdrive-mtls rule; teardown-ungated-by-DriverType regression lock, :2038; \
         Tier-3 metal-deferred)"
    );
}
