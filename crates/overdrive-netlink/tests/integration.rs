//! Integration-test entrypoint for real host-netlink adapter contracts.

#![cfg(feature = "integration-tests")]
#![allow(clippy::doc_markdown, clippy::expect_used, clippy::print_stderr)]

mod integration {
    mod bridge_guard_lifecycle;
}
