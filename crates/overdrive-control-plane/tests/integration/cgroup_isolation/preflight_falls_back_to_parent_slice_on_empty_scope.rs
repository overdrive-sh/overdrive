//! Step 01-01 (bugfix `fix-cgroup-preflight-scope-vs-slice`) — oracle
//! for ADR-0028 §4 step 4 *parent-slice fallback* clause.
//!
//! ADR-0028 §4 step 4 prescribes: inspect the enclosing slice's
//! `cgroup.subtree_control` "or the parent's if the file is empty".
//! The current implementation reads the discovered path's
//! `subtree_control` once with no fallback. When `overdrive serve` is
//! invoked from an interactive TTY, the discovered path is a leaf
//! scope (e.g. `session-3.scope`) whose `subtree_control` is empty by
//! design — controllers live in the parent `user-1000.slice`'s
//! `subtree_control`. The buggy code reads the empty leaf file,
//! finds neither `cpu` nor `memory`, and returns `DelegationMissing`
//! despite the parent slice carrying proper delegation.
//!
//! This oracle pins the desired shape:
//!
//!   <tmp>/cgroup.controllers                                   = "cpu memory io"
//!   <tmp>/user.slice/user-1000.slice/cgroup.subtree_control    = "cpu memory io"
//!   <tmp>/user.slice/.../session-3.scope/cgroup.subtree_control = ""   (leaf)
//!   <tmp>/proc-self-cgroup                                     = "`0::/user.slice/user-1000.slice/session-3.scope\n`"
//!
//! Under the buggy code path this test FAILS with
//! `DelegationMissing` because the scope's empty `subtree_control` is
//! treated as "no controllers delegated". Under the GREEN fix
//! (parent-slice fallback when the scope file is empty), the test
//! passes — the parent `user-1000.slice` carries `cpu` and `memory`,
//! so the preflight returns `Ok(())`.
//!
//! GREEN ships in step 01-02. This commit lands the RED scaffold per
//! `.claude/rules/testing.md` §RED scaffolds.

#![cfg(target_os = "linux")]

use overdrive_control_plane::cgroup_preflight::run_preflight_at;
use tempfile::TempDir;

#[test]
fn preflight_falls_back_to_parent_slice_on_empty_scope() {
    let tmp = TempDir::new().expect("tempdir");
    let cgroup_root = tmp.path();

    // Step 1 must pass: /proc/filesystems lists cgroup2.
    let proc_fs = tmp.path().join("filesystems");
    std::fs::write(&proc_fs, "nodev\tcgroup2\n").expect("write proc/filesystems");

    // Step 2 must pass: cgroup.controllers exists at the cgroupfs
    // mount root (cgroup v2 mounted).
    std::fs::write(cgroup_root.join("cgroup.controllers"), "cpu memory io\n")
        .expect("write cgroup.controllers");

    // Parent slice — user-1000.slice carries the delegation. This is
    // where systemd's `Delegate=yes` plumbing lands when the user
    // session is enrolled correctly.
    let user_slice_dir = cgroup_root.join("user.slice").join("user-1000.slice");
    std::fs::create_dir_all(&user_slice_dir).expect("create user-1000.slice dir");
    std::fs::write(user_slice_dir.join("cgroup.subtree_control"), "cpu memory io\n")
        .expect("write user-slice subtree_control");

    // Leaf scope — session-3.scope under the parent slice. Its
    // subtree_control is empty by design: scopes are leaves, they do
    // not delegate further. The buggy preflight reads this file,
    // finds no `cpu` and no `memory`, and returns DelegationMissing.
    let session_scope_dir = user_slice_dir.join("session-3.scope");
    std::fs::create_dir_all(&session_scope_dir).expect("create session-3.scope dir");
    std::fs::write(session_scope_dir.join("cgroup.subtree_control"), "")
        .expect("write empty session-scope subtree_control");

    // /proc/self/cgroup analogue — the unified-hierarchy line points
    // the discovery logic at the leaf scope. This mirrors the shape
    // an interactive `overdrive serve` from a TTY in
    // `session-3.scope` actually sees.
    let proc_self_cgroup = tmp.path().join("proc-self-cgroup");
    std::fs::write(&proc_self_cgroup, "0::/user.slice/user-1000.slice/session-3.scope\n")
        .expect("write proc/self/cgroup");

    let result = run_preflight_at(cgroup_root, /* uid = */ 1000, &proc_fs, &proc_self_cgroup);

    assert!(
        result.is_ok(),
        "preflight must fall back to the parent slice when the discovered \
         scope's cgroup.subtree_control is empty: parent user-1000.slice \
         carries cpu+memory, so the check should pass. Got: {result:?}",
    );
}
