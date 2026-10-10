//! Shared real-kernel TAP fixtures for the `overdrive-netlink` integration
//! cases: scratch TAPs named outside the production `ovd-tp-` family, each
//! deleted by RAII on every exit path. The `overdrive-netlink` integration
//! binary is in the `host-kernel-shared` nextest group.

use overdrive_netlink::{Client, TapLinkState, block_on_host_netlink, create_persistent_tap};

/// Fail closed unless the test process is root (the Lima runner's uid).
pub(super) fn require_root(test: &str) {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    let euid = unsafe { libc::geteuid() };
    assert_eq!(euid, 0, "{test} creates kernel TAPs and must run as root (cargo xtask lima run)");
}

/// A scratch-TAP name unique to this process and case, outside `ovd-tp-`.
pub(super) fn scratch_name(tag: &str) -> String {
    let name = format!("nd{tag}{:06}", std::process::id() % 1_000_000);
    assert!(name.len() < libc::IFNAMSIZ, "scratch TAP name {name} fits IFNAMSIZ");
    name
}

/// The persistent-TAP state `name` reads back through rtnetlink.
fn tap_state(name: &str) -> TapLinkState {
    let owned = name.to_owned();
    block_on_host_netlink(|| async move { Client::new()?.observe_persistent_tap(&owned).await })
        .unwrap_or_else(|error| panic!("reading back TAP {name}: {error}"))
}

/// A persistent single-queue TAP owned by the root launcher (uid 0), created
/// down, and deleted when dropped.
pub(super) struct ScratchTap {
    name: String,
}

impl ScratchTap {
    pub(super) fn create(tag: &str) -> Self {
        let name = scratch_name(tag);
        // A crashed earlier run of this pid is not possible, but a leftover
        // device under the same name would make every assertion meaningless.
        assert_eq!(tap_state(&name), TapLinkState::Absent, "scratch TAP {name} must not pre-exist");
        create_persistent_tap(&name, 0)
            .unwrap_or_else(|error| panic!("creating scratch TAP {name}: {error}"));
        Self { name }
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for ScratchTap {
    fn drop(&mut self) {
        delete_link(&self.name);
    }
}

/// Deletes a scratch name that must stay absent, should a failing case have
/// left a device behind.
pub(super) struct AbsentNameGuard(pub(super) String);

impl Drop for AbsentNameGuard {
    fn drop(&mut self) {
        delete_link(&self.0);
    }
}

/// Delete `name` if present; a failure is logged, never absorbed silently.
fn delete_link(name: &str) {
    let owned = name.to_owned();
    let deleted = block_on_host_netlink(|| async move { Client::new()?.del_link(&owned).await });
    if let Err(error) = deleted {
        eprintln!("cleanup: deleting scratch link {name} failed: {error}");
    }
}
