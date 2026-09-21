//! S-ND295-13 seeded safety/convergence evidence for boot reclamation order.
//!
//! The test drives the real injected-driver server helper. The same Arc-backed
//! [`SimVmHostState`] is passed to production VM reclamation and to the existing
//! Sim shared owner, which snapshots it inside the actual `sweep_stale` port
//! call. The current sweep-before-reclamation source order therefore leaves the
//! seeded VM-exclusive residue visible and produces a seed-bearing RED.

#![allow(clippy::doc_markdown, clippy::expect_used, clippy::print_stderr)]

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use overdrive_control_plane::dataplane_config::DataplaneConfig;
    use overdrive_control_plane::guest_network::GuestNetworkOperation;
    use overdrive_control_plane::{ServerConfig, run_server_with_obs_and_driver};
    use overdrive_core::guest_network::GuestNetworkExecWiring;
    use overdrive_core::id::{AllocationId, NodeId};
    use overdrive_core::traits::driver::{Driver, DriverType};
    use overdrive_core::traits::observation_store::ObservationStore;
    use overdrive_core::traits::vm_host_state::VmHostState;
    use proptest::prelude::*;

    use crate::adapters::SimKek;
    use crate::adapters::dataplane::SimDataplane;
    use crate::adapters::driver::SimDriver;
    use crate::adapters::guest_network::SimSharedGuestNetworkOwner;
    use crate::adapters::observation_store::SimObservationStore;
    use crate::adapters::vm_host_state::SimVmHostState;

    async fn run_seed(seed: u64) {
        eprintln!(
            "seed={seed} invariant=reclamation_completes_before_stale_shared_network_sweep_for_every_seeded_prior_vm"
        );
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let data_dir = tmp.path().join("data");
        let operator_config_dir = tmp.path().join("conf");
        std::fs::create_dir_all(&data_dir).expect("data dir");
        std::fs::create_dir_all(&operator_config_dir).expect("config dir");
        let config = ServerConfig {
            bind: "127.0.0.1:0".parse().expect("loopback bind"),
            data_dir,
            operator_config_dir,
            dataplane: Some(DataplaneConfig {
                client_iface: "lo".to_owned(),
                backend_iface: "lo".to_owned(),
            }),
            dataplane_override: Some(Arc::new(SimDataplane::new())),
            ..ServerConfig::new(Arc::new(SimKek::for_boot()))
        };

        let host = SimVmHostState::new();
        let allocation_count = usize::try_from(seed % 4 + 1).expect("1..=4 fits usize");
        let mut allocations = Vec::with_capacity(allocation_count);
        for index in 0..allocation_count {
            let alloc = AllocationId::new(&format!("nd295-s13-{seed:016x}-{index}"))
                .expect("seeded allocation id");
            if (seed.rotate_right(u32::try_from(index).unwrap_or(0)) & 1) == 1 {
                host.set_scope(alloc.clone(), BTreeSet::new());
            }
            host.set_run_dir(alloc.clone());
            host.set_clone(
                alloc.clone(),
                PathBuf::from(format!("/var/lib/overdrive/vm-clones/{alloc}.img")),
            );
            allocations.push(alloc);
        }

        let owner = Arc::new(SimSharedGuestNetworkOwner::with_sweep_host_state(host.clone()));
        let owner_port: Arc<dyn overdrive_control_plane::guest_network::SharedGuestNetworkOwner> =
            owner.clone();
        let host_port: Arc<dyn VmHostState> = Arc::new(host);
        let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
            NodeId::new("nd295-s13-node").expect("node id"),
            0,
        ));
        let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));
        let wiring = GuestNetworkExecWiring::new(Arc::clone(&config.clock));

        let handle =
            run_server_with_obs_and_driver(config, obs, driver, host_port, owner_port, wiring)
                .await
                .unwrap_or_else(|error| {
                    panic!("seed={seed}: production boot failed before oracle: {error}")
                });

        let calls = owner.calls();
        let sweep_calls = owner.sweep_calls();
        assert_eq!(sweep_calls.len(), 1, "seed={seed}: exactly one actual sweep port call");
        let sweep = &sweep_calls[0];
        assert_eq!(
            calls.get(sweep.call_index),
            Some(&GuestNetworkOperation::CleanupComplement),
            "seed={seed}: sweep snapshot must name its CleanupComplement call"
        );
        for alloc in allocations {
            assert!(
                !sweep.host.scopes.contains_key(&alloc),
                "seed={seed}: {alloc} scope remained at stale shared-network sweep"
            );
            assert!(
                !sweep.host.run_dirs.contains(&alloc),
                "seed={seed}: {alloc} run directory remained at stale shared-network sweep"
            );
            assert!(
                !sweep.host.clones.contains_key(&alloc),
                "seed={seed}: {alloc} clone remained at stale shared-network sweep"
            );
        }
        handle.shutdown(Duration::from_secs(2)).await.expect("clean seeded server shutdown");
    }

    proptest! {
        /// S-ND295-13 — old VM owners are reclaimed before stale attachment sweep.
        /// CONTRACT_SHAPE: bounded-change.
        #[test]
        fn reclamation_completes_before_stale_shared_network_sweep_for_every_seeded_prior_vm(
            seed in any::<u64>(),
        ) {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("seeded invariant runtime")
                .block_on(run_seed(seed));
        }
    }
}
