//! Tier-3 suite of the in-kernel flow forwarder `ovd_fwd`, driven through
//! its kernel adapter `overdrive_fwd::KernelFlowForwarder` (EVMOD-LIFE,
//! EVMOD-CTL, EVMOD-REC, EVMOD-END, EVMOD-PAIR, EVMOD-ATTACH-END,
//! EVMOD-DGRAM, EVMOD-KTLS, EVMOD-WQ, EVMOD-AB; ADR-0197 to ADR-0216).
//!
//! **Kernel requirement:** Linux 7.0 with this workspace's `ovd_fwd` build
//! for the running release loaded (FWD-PREREQ: the lane setup runs `cargo
//! xtask kmod-build` then `cargo xtask kmod-load`) and `vsock_loopback`
//! available. The unload cell (`unload.rs`, V-EV-UNLOAD) unloads and reloads
//! the module, so it is compiled only with `kvm-tests` and runs on the
//! native metal host under its lease.
//! Every case runs on the **host role**: both halves of a pair are host
//! sockets, the vsock halves on the loopback transport (CID 1), which
//! delivers through `virtio_transport_recv_pkt` like vhost
//! (`net/vmw_vsock/vsock_loopback.c:142 @ v7.0`), the transport the
//! forwarder probe itself runs on.
//!
//! Run: `cargo xtask lima run -- cargo nextest run -p overdrive-fwd
//! --features integration-tests`. The whole binary belongs to the
//! `host-kernel-shared` nextest group: some cases unload `vsock_loopback`,
//! mount over `/dev` in a private namespace, or measure node-wide slab.
//!
//! Every case is pending DELIVER step 4 ("Forwarder module and adapter").
//!
//! # Falsification notes (the spike's negative controls)
//!
//! The negative controls of § *Source, build and test* are not built as
//! module variants. Each names the defect a case below must catch; a module
//! that carried the defect fails that case:
//!
//! - **hazard 1** (every vsock end forwarded as a FIN): the abort cases of
//!   `stream_end` and `stream_battery` (`cut`, `rst`, `kill`) read EOF where
//!   a reset is required.
//! - **hazard 2** (classify from accumulated `peer_shutdown` at the EOF
//!   read): `stream_end::clean_full_end_…` aborts a clean flow, and
//!   `a_half_close_then_a_close_while_this_side_still_sends_is_an_abort`
//!   reads clean.
//! - **hazards 4 and 8** (watch record queued from the work item / after the
//!   original callback): `watch::the_record_is_queued_before_the_holder_observes_the_end`.
//! - **hazard 16** (no `sk_error_report` rule on a TCP half after its FIN):
//!   `stream_end::a_tcp_reset_after_a_fin_aborts_with_epipe_…` waits forever.
//! - **`h_lateshut`** (own-shut flag published after the SHUTDOWN left):
//!   `stream_end::clean_full_end_…` (270 of 300 aborted in P-71).
//! - **`h_noseed`** (no attach-time classification): the `attach_watch`
//!   cases with an end already present at attach.
//! - **`h_strict`** (`ESTABLISHED`-only attach): `attach_watch::a_tcp_half_in_close_wait_…`.
//! - **truncated-frame control** (`trunc_fix=0`): `datagram::a_truncated_frame_at_a_clean_peer_end_…`.
//! - **no-discard control** (guest slot release without the queue discard):
//!   an owner rule of `overdrive-init`, not a forwarder property; covered by
//!   the guest owner's suites.
//!
//! # Not on the host role
//!
//! The vhost device release's orphan sweep (`drivers/vhost/vsock.c:721-750
//! @ v7.0`) and the battery on the guest side of a real VM boundary need a
//! vhost peer; the loopback transport has no device release. Those cases live
//! in `guest_boundary.rs`, compiled only with `kvm-tests` and run on the
//! native `x86_64` metal host (`cargo xtask metal run -- cargo nextest run -p
//! overdrive-fwd --features integration-tests,kvm-tests`) over the
//! `overdrive-testing` guest-kit microVM fixture.

#![cfg(feature = "integration-tests")]
#![cfg(target_os = "linux")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::missing_const_for_fn,
    clippy::doc_markdown,
    clippy::similar_names,
    clippy::many_single_char_names,
    clippy::needless_pass_by_value,
    clippy::items_after_statements,
    clippy::significant_drop_tightening,
    clippy::large_stack_arrays,
    reason = "Tier-3 test code: fixtures panic on a broken precondition by design"
)]

mod integration {
    mod support;

    mod adapter_equivalence;
    mod attach_watch;
    mod context_release;
    mod datagram;
    mod end_record_abi;
    // The guest-side cases across a real VM boundary: the guest-kit microVM
    // fixture, native x86_64 metal (`cargo xtask metal run --`).
    #[cfg(feature = "kvm-tests")]
    mod guest_boundary;
    mod paused_and_stall;
    mod probe_gold;
    mod stream_battery;
    mod stream_end;
    // V-EV-UNLOAD: unloads the module, so only on the native metal host
    // under its exclusive lease (never the shared Lima VM).
    #[cfg(feature = "kvm-tests")]
    mod unload;
    mod watch;
}
