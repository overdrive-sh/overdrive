// P4 — `prefix_landing` Quint conformance (the landing-class oracle table).
//
// netns-density-295. `specs/quint/guest-flow-owner/prefix_landing.qnt` is a
// **pure, stateless definition module** (`lands(net, client, port)`) shared by
// `prefix_boot` / `intake` / `shared_table`; it has no `init`/`step`, so it is
// NOT a trace replay. Conformance is the Hebert MODELING (oracle) pattern: for
// every representative `Net` configuration × client class × port, the model's
// `lands(...)` is the oracle, and the production steering + nft landing MUST
// fall in the same class.
//
// Client classes (feature-delta / prefix_landing.qnt): `marked` (Overdrive's
// leg-S / probes), `unmarked` (any other host-local client), `remote`.
// Outcome classes: intake | legC | refused | wildcard (host-service exposure)
// | elsewhere (routed off the node). The production landing is observed at the
// node's kernel surface (real `lo` / device connects + nft + the pinned
// steering); the model classification is read by evaluating `lands` through
// quint-connect's definition-eval surface (the spec referenced IN PLACE).
//
// The load-bearing conformance claims (assumptions K-D1/K-D2/K-D4,
// K-L1/K-L2/K-L5/K-L10 — discharged by V-19 and V-26):
//   * steer=ours + route=local + loopback + entry present + listener open → intake
//   * steer=ours + no entry (or no listener) → refused (never `wildcard`: the
//     B-2 exposure trap is closed; INTAKE-BIND means no lookup finds a listener
//     by its bound address)
//   * unmarked/remote + divert + legC socket → legC (K-L2: no lookup)
//   * unmarked/remote + reject → refused
//   * route=absent → elsewhere
//   * remote ingress is never assigned to an intake (K-L5) unless `assignRemote`
//
// Spawns the `quint` CLI → gated behind `integration-tests`, run via
// `cargo xtask lima run --`.

// GATED — compiles to an EMPTY crate until DELIVER lands the production steering
// + nft landing surface and the shared quint-connect base. None exists yet;
// none invented here (CLAUDE.md § "Implement to the design"). DELIVER removes
// the `#![cfg(any())]` gate when the surface lands (co-sequenced per
// shared-test-infrastructure.md § 0 / § 6). `cfg(any())` = always-false,
// no feature name, no `unexpected_cfgs` warning.
#![cfg(any())]

use overdrive_system_conformance::quint_driver::{QuintDef, eval_landing_model};
use overdrive_system_conformance::vsock_harness::{
    LandingProbe, VsockNodeConfig, boot_landing_probe,
};

const SPEC: &str = "specs/quint/guest-flow-owner/prefix_landing.qnt";

/// One `Net` configuration (mirrors the `prefix_landing.qnt` `Net` record) the
/// production node is driven into, so the model and the kernel decide the same
/// inputs.
#[derive(Clone, Debug)]
struct NetConfig {
    marked_clients_exist: bool, // serve is serving
    leg_c_socket: bool,
    divert: bool, // nft prefix-divert to leg-C
    reject: bool, // nft reject/drop of non-exempt prefix traffic
    route_local: bool,
    steer_ours: bool, // the pinned sk_lookup program is attached
    entry_ports: &'static [u16],
    listening_ports: &'static [u16],
}

/// The representative configurations that cover every branch of `lands`.
fn landing_configs() -> Vec<NetConfig> {
    vec![
        // serving: steered + listening + route local + rules present
        NetConfig {
            marked_clients_exist: true,
            leg_c_socket: true,
            divert: true,
            reject: true,
            route_local: true,
            steer_ours: true,
            entry_ports: &[22],
            listening_ports: &[22],
        },
        // steered but no entry -> refused (never wildcard, B-2 closed)
        NetConfig {
            marked_clients_exist: true,
            leg_c_socket: true,
            divert: true,
            reject: true,
            route_local: true,
            steer_ours: true,
            entry_ports: &[],
            listening_ports: &[22],
        },
        // unmarked/remote + divert + legC -> legC
        NetConfig {
            marked_clients_exist: true,
            leg_c_socket: true,
            divert: true,
            reject: false,
            route_local: true,
            steer_ours: true,
            entry_ports: &[],
            listening_ports: &[],
        },
        // no tagged route -> elsewhere
        NetConfig {
            marked_clients_exist: true,
            leg_c_socket: false,
            divert: false,
            reject: false,
            route_local: false,
            steer_ours: false,
            entry_ports: &[],
            listening_ports: &[],
        },
        // serve down: steering present, route local, nothing serving -> refused
        NetConfig {
            marked_clients_exist: false,
            leg_c_socket: false,
            divert: true,
            reject: true,
            route_local: true,
            steer_ours: true,
            entry_ports: &[],
            listening_ports: &[],
        },
    ]
}

const CLIENTS: [&str; 3] = ["marked", "unmarked", "remote"];
const PORTS: [u16; 2] = [22, 2222];

/// Oracle-table conformance: the model classifies, the kernel must agree.
#[quint_run(spec = SPEC)]
fn production_landing_matches_the_model_for_every_client_class_and_config() {
    let model: QuintDef = eval_landing_model(SPEC); // quint-connect evaluates `lands` in place
    for cfg in landing_configs() {
        let probe: LandingProbe = boot_landing_probe(VsockNodeConfig::from_net(&cfg));
        for client in CLIENTS {
            for port in PORTS {
                let expected = model.lands(&cfg, client, port); // "intake"|"legC"|"refused"|"wildcard"|"elsewhere"
                let observed = probe.land(client, port); // real connect, classified at the kernel
                assert_eq!(
                    observed, expected,
                    "prefix_landing: client={client} port={port} cfg={cfg:?}: \
                     production landed {observed}, model says {expected}",
                );
                // The exposure trap, asserted explicitly: the model never says
                // `wildcard` while the steering is attached (B-2 closed), and
                // neither may the kernel.
                if cfg.steer_ours {
                    assert_ne!(
                        observed, "wildcard",
                        "a host service is never reached while steering is attached"
                    );
                }
            }
        }
    }
}
