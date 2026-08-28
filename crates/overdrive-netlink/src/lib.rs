//! `overdrive-netlink` — subprocess-free dataplane provisioning over netlink
//! (ADR-0085 D2). The single auditable home for the rtnetlink client, the
//! hand-rolled ethtool `FEATURES_SET` encoder, the `setns` helper, and the
//! shared errno-carrying [`NetlinkError`].
//!
//! `adapter-host`-class: it performs real kernel I/O (rtnetlink over
//! `NETLINK_ROUTE`, a raw `NETLINK_GENERIC` socket for ethtool, and — from
//! slice 01-02 — `setns` + `/proc/sys`). It exposes **plain impl modules** —
//! NO port trait, NO `Sim` adapter (that is the deferred #197 continuous
//! network-reconciler, ADR-0085 D9). It invokes **no** named infra-CLI
//! subprocess (`ip` / `ethtool` / `sysctl` / …): every operation is a
//! syscall / netlink message, which is the whole point of the swap.
//!
//! Consumed by `overdrive-control-plane` (`veth_provisioner`, from slice
//! 01-01) and `overdrive-worker` (`mtls_intercept`, from slice 02-01).

// Tests panic-on-None/Err as the intended oracle (mirrors overdrive-core's
// crate-level convention).
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

pub mod client;
pub mod error;
pub mod ethtool;
pub mod nft;
pub mod runtime;
pub mod setns;

pub use client::{Client, create_persistent_tap};
pub use error::{NetlinkError, errno_is_idempotent};
pub use runtime::{block_on_host_netlink, block_on_netlink};
pub use setns::in_netns;
