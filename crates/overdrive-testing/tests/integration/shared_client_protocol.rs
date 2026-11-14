//! The client half of the shared-guest protocol at its driving port,
//! `attach_shared(&SharedEndpoint, &GuestPlacement, test_name)`: the test builds
//! the endpoint itself (socket, lock and owner-binary paths in a temporary
//! directory), so no guest boots and no node-global state is touched.
//!
//! Placement on activation: `#[overdrive_test(host_pure)]` (no guest, no
//! node-global effect); the form is given by the slice that activates these
//! tests and no placement-table entry is ever added.

use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use overdrive_testing::placement::{
    GuestPlacement, RunInGuestError, SharedEndpoint, Sharing, attach_shared,
};

const TEST_NAME: &str = "integration::shared_client_protocol::a_test_of_the_shared_module";

/// A scratch directory unique to this process and scenario, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir()
            .join(format!("overdrive-shared-client-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("creating the scratch directory");
        Self(dir)
    }

    fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
    }

    /// An executable that does nothing, standing in for the owner binary.
    fn executable(&self, file: &str) -> PathBuf {
        let path = self.path(file);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").expect("writing the stand-in executable");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("making the stand-in executable");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const fn shared_placement() -> GuestPlacement {
    GuestPlacement::declare(
        "cgroup-default",
        "base",
        Sharing::Module,
        "integration::shared_client_protocol",
    )
}

fn endpoint_in(scratch: &Scratch, owner_binary: &Path) -> SharedEndpoint {
    SharedEndpoint {
        socket: scratch.path("owner.sock"),
        lock: scratch.path("owner.lock"),
        owner_binary: owner_binary.to_path_buf(),
    }
}

/// An owner binary that is not there is an unprepared kit, refused before the
/// election lock is taken. CONTRACT_SHAPE: bounded-change (the attach result
/// and the lock file are the whole observable change).
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_missing_owner_binary_is_refused_as_an_unprepared_kit_before_the_election_lock_is_taken() {
    let scratch = Scratch::new("missing-owner");
    let absent = scratch.path("overdrive-shared-guest-owner-that-is-not-built");
    let endpoint = endpoint_in(&scratch, &absent);

    let outcome = attach_shared(&endpoint, &shared_placement(), TEST_NAME);

    let error = outcome.expect_err("a missing owner binary does not attach");
    assert!(
        matches!(error, RunInGuestError::LaneNotPrepared { .. }),
        "an unprepared kit, not an execution failure: {error}"
    );
    assert!(
        error.to_string().contains(absent.to_str().expect("utf-8 path")),
        "the refusal names the owner binary path: {error}"
    );
    assert!(!endpoint.lock.exists(), "the election lock was not created");
}

/// A connection cut before the acknowledgement ran nothing, so the client
/// re-attaches once through the election; a second cut is owner loss.
/// CONTRACT_SHAPE: bounded-change (connections accepted and the failure).
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_connection_cut_before_the_acknowledgement_re_attaches_once_and_a_second_cut_is_owner_loss() {
    let scratch = Scratch::new("cut-connection");
    let owner_binary = scratch.executable("overdrive-shared-guest-owner");
    let endpoint = endpoint_in(&scratch, &owner_binary);

    // A listener that accepts every connection and closes it unread.
    let listener = UnixListener::bind(&endpoint.socket).expect("binding the endpoint socket");
    listener.set_nonblocking(true).expect("a pollable listener");
    let accepted = Arc::new(AtomicU32::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let server = {
        let (accepted, stop) = (Arc::clone(&accepted), Arc::clone(&stop));
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((connection, _)) => {
                        accepted.fetch_add(1, Ordering::SeqCst);
                        drop(connection);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        })
    };

    let outcome = attach_shared(&endpoint, &shared_placement(), TEST_NAME);

    stop.store(true, Ordering::SeqCst);
    server.join().expect("the listener thread ends");
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        2,
        "the first attach and exactly one re-attach reached the listener"
    );
    let error = outcome.expect_err("a cut connection never passes");
    match &error {
        RunInGuestError::GuestExecution { detail, .. } => {
            assert!(detail.contains("owner"), "the failure names the owner stage: {detail}");
        }
        other => panic!("the second cut is owner loss (GuestExecution), found: {other}"),
    }
}
