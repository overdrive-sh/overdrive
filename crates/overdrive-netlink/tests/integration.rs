//! Integration-test entrypoint for real host-netlink adapter contracts.

#![cfg(feature = "integration-tests")]
#![allow(clippy::doc_markdown, clippy::expect_used, clippy::print_stderr)]

mod integration {
    mod bridge_guard_lifecycle;
    // netns-density-295 S-ND295-72 (E22 (a) and (b)): a bridge
    // `ensure_bridge` creates carries its address from creation; a present
    // link of any kind is adopted without a write.
    mod managed_link_address;
    // netns-density-295 S-ND295-38 / S-ND295-39 (D-295-R2, R4) and S-ND295-49
    // (D-295-R22 read-back): the TAP queue attach port and the TAP debug
    // message mask against real kernel TAPs.
    mod tap_debug_msg_mask;
    mod tap_queue_attach;
}
