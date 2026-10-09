// P2 — `owner_flows` quint-connect conformance driver (DISTILL, netns-density-295).
//
// Replays traces generated from `specs/quint/guest-flow-owner/owner_flows.qnt`
// (instances `flows_ok`, `flows_beacon`, `flows_half`) through the PRODUCTION
// guest-flow owner + VM-driver beacon, projecting `State` from the owner's
// observable session/flow surface and comparing after every step
// (`.claude/rules/testing.md` §"Quint conformance"; ADR-0168). Defends
// `owner_flows` invariants 1, 3, 4, 4+, 9, 13. The spec is referenced IN PLACE
// and never copied; `checks.toml` is NOT edited (it is hashed into the
// evidence). This driver is the implementation-vs-model lane; the invariant /
// witness / hazard checks in `checks.toml` are the separate model-check lane
// (`cargo xtask quint check`).
//
// GATING — the whole body lives in a `#[cfg(any())]` module (always false) so
// `--all-targets` stays green while it is authored, until the DELIVER
// roadmap-reconciliation lands:
//   * the shared `overdrive_system_conformance::vsock_harness` base
//     (`boot_vsock_node` + `ConformanceNode` observable surface), and
//   * the shared `overdrive_system_conformance::quint_driver` base (the
//     quint-connect wiring scaffold each module driver specialises), and
//   * the production owner's observable session/flow surface.
// DELIVER removes the `#[cfg(any())]` wrapper (gating instead behind
// `integration-tests` + Lima + the `host-kernel-shared` nextest group: the
// driver spawns the `quint` CLI and drives the composed owner). CLAUDE.md
// §"Implement to the design": no production API is invented — the harness /
// observable surface are the DISTILL-shaped test-support contract (gap #4).

#[cfg(any())]
mod pending_deliver {
    #![allow(dead_code, unused_variables)]

    use overdrive_system_conformance::quint_driver::QuintNodeBase;
    use overdrive_system_conformance::vsock_harness::{ConformanceNode, boot_vsock_node};
    use quint_connect::{Config, Driver, Result, State, Step, quint_run, switch};
    use serde::Deserialize;

    // -------------------------------------------------------------------------
    // State projection — the subset of the `owner_flows` state space this driver
    // compares, deserialised from the ITF trace AND read from the owner's
    // observable surface. serde ignores spec fields not named here, so the
    // driver pins exactly the observable contract: the CID phase, the lease
    // generation, whether the node is quiesced, the claim holder, and the ghost
    // witnesses the invariants assert on. No private owner field is read.
    // -------------------------------------------------------------------------

    #[derive(Debug, PartialEq, Eq, Deserialize)]
    struct OwnerFlowsState {
        /// `Absent | Provisioned | Active | Retiring` (owner CID table).
        alloc: String,
        /// The lease currently holding the CID (its launch token / generation).
        // `r#gen` — `gen` is a reserved keyword in edition 2024; the raw
        // identifier keeps the serde key literally `gen` (the Quint model
        // variable name), so the model↔driver projection stays bit-identical.
        r#gen: i64,
        /// Node admission latch.
        quiesced: bool,
        /// The connection holding the CID's session claim (`0` = none).
        claim: i64,
        /// Ghost: a `Paired` was emitted while its admission conditions did not
        /// hold (inv 4).
        #[serde(rename = "badPaired")]
        bad_paired: bool,
        /// Ghost: a control message was sent for a flow of another launch (4+).
        #[serde(rename = "crossMsg")]
        cross_msg: bool,
        /// Ghost: a session was refused because another launch's connection held
        /// the claim (4+).
        #[serde(rename = "heldByOther")]
        held_by_other: bool,
        /// Ghost: a closed host flow was resurrected (U-5 / inv 9).
        resurrected: bool,
    }

    impl State<OwnerFlowsDriver> for OwnerFlowsState {
        fn from_driver(driver: &OwnerFlowsDriver) -> Result<Self> {
            // Project from the owner's observable surface — never a private field.
            let obs = driver.node.observe_owner_flows()?;
            Ok(Self {
                alloc: obs.cid_phase,
                r#gen: obs.lease_generation,
                quiesced: obs.quiesced,
                claim: obs.session_claim_holder.map_or(0, |c| c as i64),
                bad_paired: obs.ghost_bad_paired,
                cross_msg: obs.ghost_cross_launch_message,
                held_by_other: obs.ghost_session_held_by_other,
                resurrected: obs.ghost_resurrected,
            })
        }
    }

    // -------------------------------------------------------------------------
    // The driver: one production composition per trace, stepped by the spec's
    // actions through the shared harness seam. Each arm forwards to a
    // `ConformanceNode` method; the driver holds no business logic (Mandate-12).
    // -------------------------------------------------------------------------

    struct OwnerFlowsDriver {
        node: ConformanceNode,
    }

    impl OwnerFlowsDriver {
        fn new() -> Self {
            // Boots the server composition with the vsock transport wired and
            // `Sim*` at the external / non-deterministic driven ports (Pillar 3).
            Self { node: boot_vsock_node(Default::default()) }
        }
    }

    impl Driver for OwnerFlowsDriver {
        type State = OwnerFlowsState;

        fn config() -> Config {
            // Top-level state; nondet via Quint's builtin `mbt` variables.
            Config::default()
        }

        fn step(&mut self, step: &Step) -> Result {
            // The action names mirror the `owner_flows` module's actions; each
            // maps to the owner's driving-port call through the shared harness
            // seam. The `c` / `f` / `h` nondet picks are the connection / flow /
            // host-flow ids the spec chose.
            switch!(step {
                init => self.node.reset_owner_flows()?,
                Provision => self.node.provision()?,
                VmStart => self.node.vm_start()?,
                VmStop => self.node.vm_stop()?,
                Activate => self.node.activate()?,
                Teardown => self.node.teardown()?,
                Release => self.node.release()?,
                Quiesce => self.node.quiesce()?,
                Restore => self.node.restore()?,
                // Control session (flows_ok; inv 1, 4+).
                GuestOpenSession(c: u64) => self.node.guest_open_session(c)?,
                HostAcceptSession(c: u64) => self.node.host_accept_session(c)?,
                HostReadOpen(c: u64) => self.node.host_read_open(c)?,
                HostClaimSession(c: u64) => self.node.host_claim_session(c)?,
                HostSessionLoss(c: u64) => self.node.host_session_loss(c)?,
                HostDeadSessionLoss(c: u64) => self.node.host_session_loss(c)?,
                // Guest-opened flow admission (inv 3, 4, 4+).
                GuestOpenFlow(f: u64) => self.node.guest_open_flow(f)?,
                HostAcceptFlow(f: u64) => self.node.host_accept_flow(f)?,
                HostCheckFlow(f: u64) => self.node.host_check_flow(f)?,
                HostRegisterFlow(f: u64) => self.node.host_register_flow(f)?,
                HostReadReq(f: u64) => self.node.host_read_request(f)?,
                HostInstall(f: u64) => self.node.host_install(f)?,
                HostStartConnect(f: u64) => self.node.host_start_connect(f)?,
                HostConnectDone(f: u64) => self.node.host_connect_done(f)?,
                HostConnectDiscard(f: u64) => self.node.host_connect_discard(f)?,
                HostFlowEnds(f: u64) => self.node.host_flow_ends(f)?,
                // Host-opened TcpAccept, both sides (inv 9, K-A4).
                HostOpenAccept(h: u64) => self.node.host_open_accept(h)?,
                GuestAcceptV(h: u64) => self.node.guest_accept_v(h)?,
                GuestPairOrRefuse(h: u64) => self.node.guest_pair_or_refuse(h)?,
                HostRecvPaired(h: u64) => self.node.host_recv_paired(h)?,
                HostRecvRefused(h: u64) => self.node.host_recv_refused(h)?,
                HostAbortAccept(h: u64) => self.node.host_abort_accept(h)?,
                GuestRecvAbort(h: u64) => self.node.guest_recv_abort(h)?,
                GuestSeesHostClose(h: u64) => self.node.guest_sees_host_close(h)?,
                GuestLossAbort(h: u64) => self.node.guest_loss_abort(h)?,
                GuestQueueReset(h: u64) => self.node.guest_queue_reset(h)?,
                _ => {
                    // An unmapped action is an authoring gap, surfaced loudly
                    // rather than silently passed.
                    anyhow::bail!("unmapped flows_ok action: {}", step.action_taken);
                }
            })
        }
    }

    struct BeaconDriver {
        node: ConformanceNode,
    }

    impl Driver for BeaconDriver {
        type State = ();

        fn step(&mut self, step: &Step) -> Result {
            switch!(step {
                init => self.node.reset_beacon()?,
                Provision => self.node.provision()?,
                VmStart => self.node.vm_start()?,
                Teardown => self.node.teardown()?,
                Release => self.node.release()?,
                DriverStop => self.node.driver_stop()?,
                GuestOpenBeacon(b: u64) => self.node.guest_open_beacon(b)?,
                HostAcceptBeacon(b: u64) => self.node.host_accept_beacon(b)?,
                HostReadReady(b: u64) => self.node.host_read_ready(b)?,
                _ => anyhow::bail!("unmapped flows_beacon action: {}", step.action_taken),
            })
        }
    }

    struct HalfCloseDriver {
        node: ConformanceNode,
    }

    impl Driver for HalfCloseDriver {
        type State = ();

        fn step(&mut self, step: &Step) -> Result {
            // The half-close handshake per side (HALFCLOSE-SIGNAL, inv 13). The
            // shared quint_driver base maps the `flows_half` HC channel actions.
            self.node.half_close_step(&step.action_taken, step)
        }
    }

    // -------------------------------------------------------------------------
    // Conformance tests. `#[quint_run]` replays randomized simulation traces of
    // each instance through the production owner, printing the seed on failure
    // (same reproducibility rule as DST). The production composition — not a
    // re-implemented protocol — is the thing under test
    // (`.claude/rules/testing.md`: a driver that re-implements the protocol is
    // testing theatre, rejected). DELIVER adds `#[quint_test]` pins for the
    // named critical-scenario runs the spec defines.
    // -------------------------------------------------------------------------

    #[quint_run(
        spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
        main = "flows_ok",
        max_samples = 50
    )]
    fn owner_flows_core_conformance() -> impl Driver {
        OwnerFlowsDriver::new()
    }

    #[quint_run(
        spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
        main = "flows_beacon",
        max_samples = 50
    )]
    fn owner_flows_beacon_conformance() -> impl Driver {
        BeaconDriver { node: boot_vsock_node(Default::default()) }
    }

    #[quint_run(
        spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
        main = "flows_half",
        max_samples = 50
    )]
    fn owner_flows_half_close_conformance() -> impl Driver {
        HalfCloseDriver { node: boot_vsock_node(Default::default()) }
    }
}
