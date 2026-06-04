//! S-04-A driving-adapter companion — `overdrive deploy <udp-spec>`
//! accepted via the direct-handler path (the deploy half that step
//! 01-03 scoped out).
//!
//! Per `crates/overdrive-cli/CLAUDE.md` § *Integration tests — no
//! subprocess*: this test calls
//! `overdrive_cli::commands::job::submit(SubmitArgs { ... })` directly
//! as a Rust async function — the in-process handler behind
//! `overdrive deploy <SPEC>` in the detached / non-TTY (JSON-ack) lane
//! (`main.rs` `Command::Deploy` → `commands::job::submit` →
//! `render::workload_submit_accepted`). NO
//! `Command::new(env!("CARGO_BIN_EXE_overdrive"))`.
//!
//! The flow this exercises:
//!
//! ```text
//! TempDir/dns-resolver.toml          (real on-disk TOML — [service] + udp [[listener]])
//!     │ WorkloadSpecInput::from_toml_str
//!     ▼
//! WorkloadSpecInput::Service          (parser-side ServiceSpec — Listener carries Proto::Udp)
//!     │ project → ServiceSpecInput → SubmitSpecInput::Service
//!     ▼
//! POST /v1/jobs (real reqwest + rustls, JSON-ack lane)
//!     │
//!     ▼
//! handlers::submit_workload                (ServiceV1::from_submit)
//!     │ archive_for_store
//!     ▼
//! LocalIntentStore (real redb) @ workloads/dns-resolver
//! ```
//!
//! The load-bearing assertion (AC #2 / C3 guard): the persisted
//! `WorkloadIntent::Service(ServiceV1)` at `workloads/dns-resolver`
//! carries a listener whose `protocol == Proto::Udp` — proving the
//! operator's `protocol = "udp"` token flowed spec → handler → intent
//! without a `Proto::Tcp` literal substitution along the way. This is
//! the driving-adapter mirror of 01-01's intent→hydrator C3 guard. If
//! the call site that wires the listener protocol through to the
//! persisted intent were reverted (e.g. dropping the Service arm in
//! `submit`, or hard-coding `Proto::Tcp`), this test goes RED.
//!
//! Together with 01-03's dataplane wire half (`REVERSE_NAT_MAP` dump +
//! VIP-sourced reply) this closes the full S-04-A walking-skeleton
//! acceptance criterion: deploy-accepted + reverse-path-VIP-sourced.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use overdrive_cli::commands::job::SubmitArgs;
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_control_plane::api::IdempotencyOutcome;
use overdrive_core::aggregate::{IntentKey, WorkloadIntent};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::WorkloadId;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

async fn spawn_server() -> (ServeHandle, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let args = ServeArgs { bind, data_dir, config_dir };
    let handle = overdrive_cli::commands::serve::run_with_dataplane(
        args,
        std::sync::Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
    )
    .await
    .expect("serve::run");
    (handle, tmp)
}

/// Path of the on-disk `LocalIntentStore` redb file the in-process
/// server writes through. Mirrors `exec_spec_walking_skeleton.rs` /
/// `<data_dir>/intent.redb`.
fn intent_redb_path(tmp: &Path) -> PathBuf {
    tmp.join("data").join("intent.redb")
}

fn config_path(tmp: &Path) -> PathBuf {
    tmp.join("conf").join(".overdrive").join("config")
}

/// A single-UDP-listener Service spec, the deploy half of S-04-A's
/// `dns-resolver.toml` (udp listener on 5353, backend bound on 5353).
const fn dns_resolver_udp_toml() -> &'static str {
    r#"
[service]
id = "dns-resolver"
replicas = 1

[exec]
command = "/opt/dns-resolver/bin/resolver"
args    = ["--listen", "5353"]

[resources]
cpu_milli = 250
memory_bytes = 67108864

[[listener]]
port = 5353
protocol = "udp"
"#
}

fn write_toml(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write toml");
    path
}

#[tokio::test]
async fn deploy_udp_service_is_accepted_and_persisted_intent_carries_proto_udp() {
    let (handle, server_tmp) = spawn_server().await;
    let server_cfg = config_path(server_tmp.path());

    // Phase 1 — write the single-UDP-listener Service spec and deploy
    // via the direct handler (the JSON-ack lane behind
    // `overdrive deploy <SPEC>`).
    let spec_path = write_toml(server_tmp.path(), "dns-resolver.toml", dns_resolver_udp_toml());
    let submit_output = overdrive_cli::commands::job::submit(SubmitArgs {
        spec: spec_path,
        config_path: server_cfg.clone(),
    })
    .await
    .expect("job::submit must accept the single-UDP-listener Service spec end-to-end");

    // Phase 2 — AC #1: the deploy is accepted and renders the
    // `workload_submit_accepted` shape ("Accepted."). This is exactly
    // what `main.rs` prints for the detached / non-TTY deploy lane.
    let rendered = overdrive_cli::render::workload_submit_accepted(&submit_output);
    assert!(
        rendered.contains("Accepted."),
        "deploy of a UDP Service must render the workload_submit_accepted \
         shape; got:\n{rendered}",
    );
    assert_eq!(
        submit_output.workload_id, "dns-resolver",
        "echoed workload_id must equal the spec id",
    );
    assert_eq!(
        submit_output.outcome,
        IdempotencyOutcome::Inserted,
        "a fresh deploy must report `outcome = Inserted` (ADR-0020)",
    );

    // Phase 3 — clean shutdown before the back-door read; redb takes an
    // exclusive lock at `Database::create` time.
    handle.shutdown().await.expect("clean shutdown");

    // Phase 4 — AC #2 / C3 guard: back-door IntentStore read. The
    // `workload_submit_accepted` render proves the deploy was accepted,
    // but proves nothing about the protocol carried through to the
    // persisted intent. Re-open the redb file the handler wrote through,
    // deserialise the `WorkloadIntent::Service(ServiceV1)` at
    // `workloads/dns-resolver`, and assert a listener carries
    // `Proto::Udp` — the spec→handler→intent path threaded the
    // operator's `protocol = "udp"` token without reaching a
    // `Proto::Tcp` literal for this service.
    let store = LocalIntentStore::open(intent_redb_path(server_tmp.path()))
        .expect("re-open intent.redb for back-door read");
    let workload_id =
        WorkloadId::from_str("dns-resolver").expect("WorkloadId::from_str(\"dns-resolver\")");
    let key = IntentKey::for_workload(&workload_id);
    let stored = store.get(key.as_bytes()).await.expect("back-door IntentStore::get must succeed");
    let bytes = stored.expect(
        "after a successful deploy the intent key `workloads/dns-resolver` MUST be \
         populated; an empty key here means the server skipped persistence",
    );

    let intent_path = intent_redb_path(server_tmp.path());
    let intent = WorkloadIntent::from_store_bytes(&bytes, &intent_path, None)
        .expect("typed codec decode of WorkloadIntent from back-door read bytes");
    let WorkloadIntent::Service(service) = intent else {
        panic!(
            "test precondition: a [service] + [[listener]] spec must persist \
             WorkloadIntent::Service, not Job/Schedule",
        );
    };

    let udp_listeners: Vec<_> =
        service.listeners.iter().filter(|l| l.protocol == Proto::Udp).collect();
    assert_eq!(
        udp_listeners.len(),
        1,
        "the persisted Service intent must carry exactly one udp listener \
         (the operator's `protocol = \"udp\"` on port 5353); a missing udp \
         listener (or a Proto::Tcp substitution) means the protocol was lost \
         on the spec → handler → intent path. listeners = {:?}",
        service.listeners,
    );
    assert_eq!(
        udp_listeners[0].port.get(),
        5353,
        "the udp listener's port must equal the operator's declared 5353",
    );
    assert!(
        !service.listeners.iter().any(|l| l.protocol == Proto::Tcp),
        "C3 guard: NO Proto::Tcp listener may appear on this UDP-only service — \
         a tcp listener here means a literal substitution leaked onto the \
         spec → handler → intent path. listeners = {:?}",
        service.listeners,
    );
}
