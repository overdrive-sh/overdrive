//! Self-test: the `std::fs::*` inside `async fn` lint clause for
//! `adapter-host`-class crates.
//!
//! Per `.claude/rules/development.md` § Concurrency & async, sync
//! `std::fs::*` inside an `async fn` body blocks the tokio worker
//! thread. Production code in `adapter-host` crates must use
//! `tokio::fs::*` (preferred) or `tokio::task::spawn_blocking` (escape
//! hatch). The dst-lint scanner enforces this at PR time.
//!
//! These tests exercise the in-process scanner API so the rule's
//! coverage is auditable without spinning up the subprocess harness:
//!
//! 1. `std::fs::write` directly inside `async fn` → flagged
//! 2. `std::fs::create_dir_all` directly inside `async fn` → flagged
//! 3. Inside an `impl` block's `async fn` method → flagged
//! 4. Inside a free `async { … }` block → flagged
//! 5. Inside a sync `fn` → NOT flagged (helpers are allowed)
//! 6. Inside a `tokio::task::spawn_blocking(|| { … })` sync closure
//!    nested inside an `async fn` → NOT flagged (escape hatch)
//! 7. Inside `#[cfg(test)]` items → NOT flagged (tests may use sync
//!    `std::fs` for fixture setup)
//!
//! Gated behind the `integration-tests` feature — see the feature
//! comment in `xtask/Cargo.toml` for rationale.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used)]

use xtask::dst_lint::scan_source_async_fs;

// -----------------------------------------------------------------------------
// Positive cases — std::fs::* inside async context must be flagged
// -----------------------------------------------------------------------------

#[test]
fn flags_std_fs_write_inside_free_async_fn() {
    let source = "\
        pub async fn save() {\n\
            std::fs::write(\"/tmp/x\", b\"hi\").unwrap();\n\
        }\n\
    ";
    let violations = scan_source_async_fs(source, "fixture.rs")
        .expect("scan_source_async_fs must succeed for valid input");
    assert!(
        !violations.is_empty(),
        "std::fs::write inside async fn must be flagged; got {violations:?}",
    );
    assert!(
        violations.iter().any(|v| v.banned_path.contains("std::fs")),
        "violation banned_path must reference std::fs; got {violations:?}",
    );
}

#[test]
fn flags_std_fs_create_dir_all_inside_async_fn() {
    let source = "\
        pub async fn make_dir() {\n\
            std::fs::create_dir_all(\"/tmp/x\").unwrap();\n\
        }\n\
    ";
    let violations =
        scan_source_async_fs(source, "fixture.rs").expect("scan_source_async_fs must succeed");
    assert!(
        !violations.is_empty(),
        "std::fs::create_dir_all inside async fn must be flagged; got {violations:?}",
    );
}

#[test]
fn flags_std_fs_inside_async_method_on_impl_block() {
    let source = "\
        pub struct Worker;\n\
        impl Worker {\n\
            pub async fn run(&self) {\n\
                std::fs::write(\"/tmp/x\", b\"hi\").unwrap();\n\
            }\n\
        }\n\
    ";
    let violations =
        scan_source_async_fs(source, "fixture.rs").expect("scan_source_async_fs must succeed");
    assert!(
        !violations.is_empty(),
        "std::fs::write inside async method on impl block must be flagged; got {violations:?}",
    );
}

#[test]
fn flags_std_fs_inside_async_move_block() {
    // `async move { … }` is the future-returning expression form; the
    // body executes on the tokio executor and must not block.
    let source = "\
        pub fn launch() -> impl core::future::Future<Output = ()> {\n\
            async move {\n\
                std::fs::write(\"/tmp/x\", b\"hi\").unwrap();\n\
            }\n\
        }\n\
    ";
    let violations =
        scan_source_async_fs(source, "fixture.rs").expect("scan_source_async_fs must succeed");
    assert!(
        !violations.is_empty(),
        "std::fs::write inside async move block must be flagged; got {violations:?}",
    );
}

#[test]
fn flags_std_fs_read_to_string_inside_async_fn() {
    let source = "\
        pub async fn read_back() -> String {\n\
            std::fs::read_to_string(\"/tmp/x\").unwrap()\n\
        }\n\
    ";
    let violations =
        scan_source_async_fs(source, "fixture.rs").expect("scan_source_async_fs must succeed");
    assert!(
        !violations.is_empty(),
        "std::fs::read_to_string inside async fn must be flagged; got {violations:?}",
    );
}

// -----------------------------------------------------------------------------
// Negative cases — sync, escape hatch, and #[cfg(test)] are permitted
// -----------------------------------------------------------------------------

#[test]
fn permits_std_fs_inside_sync_free_fn() {
    let source = "\
        pub fn save() {\n\
            std::fs::write(\"/tmp/x\", b\"hi\").unwrap();\n\
        }\n\
    ";
    let violations =
        scan_source_async_fs(source, "fixture.rs").expect("scan_source_async_fs must succeed");
    assert!(
        violations.is_empty(),
        "std::fs::write inside sync fn must be permitted (sync helpers); got {violations:?}",
    );
}

#[test]
fn permits_std_fs_inside_sync_method_on_impl_block() {
    let source = "\
        pub struct Helper;\n\
        impl Helper {\n\
            pub fn save(&self) {\n\
                std::fs::write(\"/tmp/x\", b\"hi\").unwrap();\n\
            }\n\
        }\n\
    ";
    let violations =
        scan_source_async_fs(source, "fixture.rs").expect("scan_source_async_fs must succeed");
    assert!(
        violations.is_empty(),
        "std::fs::write inside sync method must be permitted; got {violations:?}",
    );
}

#[test]
fn permits_std_fs_inside_spawn_blocking_sync_closure_inside_async_fn() {
    // The escape hatch named in the rule: a sync closure handed to
    // `tokio::task::spawn_blocking` is run on the blocking pool, not on
    // the async executor. The lexical containment in `async fn` is
    // irrelevant — what matters is the *containing fn body's* async
    // context, which the sync closure resets.
    let source = "\
        pub async fn save() {\n\
            tokio::task::spawn_blocking(|| {\n\
                std::fs::write(\"/tmp/x\", b\"hi\").unwrap();\n\
            }).await.unwrap();\n\
        }\n\
    ";
    let violations =
        scan_source_async_fs(source, "fixture.rs").expect("scan_source_async_fs must succeed");
    assert!(
        violations.is_empty(),
        "std::fs::write inside a sync closure (spawn_blocking shape) must be permitted; \
         got {violations:?}",
    );
}

#[test]
fn permits_std_fs_inside_cfg_test_async_test() {
    // `#[cfg(test)] mod tests { … }` is the documented exception in the
    // rule — test fixture setup may use sync std::fs without penalty.
    let source = "\
        #[cfg(test)]\n\
        mod tests {\n\
            #[tokio::test]\n\
            async fn check() {\n\
                std::fs::write(\"/tmp/x\", b\"hi\").unwrap();\n\
            }\n\
        }\n\
    ";
    let violations =
        scan_source_async_fs(source, "fixture.rs").expect("scan_source_async_fs must succeed");
    assert!(
        violations.is_empty(),
        "std::fs::write inside #[cfg(test)] mod must be permitted; got {violations:?}",
    );
}

#[test]
fn permits_std_fs_inside_cfg_test_attributed_fn() {
    // The `#[cfg(test)]` attribute can appear on the fn directly, not
    // just on a containing module — same exception applies.
    let source = "\
        #[cfg(test)]\n\
        #[tokio::test]\n\
        async fn check() {\n\
            std::fs::write(\"/tmp/x\", b\"hi\").unwrap();\n\
        }\n\
    ";
    let violations =
        scan_source_async_fs(source, "fixture.rs").expect("scan_source_async_fs must succeed");
    assert!(
        violations.is_empty(),
        "std::fs::write inside #[cfg(test)]-attributed fn must be permitted; got {violations:?}",
    );
}

#[test]
fn permits_tokio_fs_inside_async_fn() {
    // The recommended replacement — `tokio::fs::*` — must not itself
    // be flagged by the new clause.
    let source = "\
        pub async fn save() {\n\
            tokio::fs::write(\"/tmp/x\", b\"hi\").await.unwrap();\n\
        }\n\
    ";
    let violations =
        scan_source_async_fs(source, "fixture.rs").expect("scan_source_async_fs must succeed");
    assert!(
        violations.is_empty(),
        "tokio::fs::write inside async fn must be permitted; got {violations:?}",
    );
}
