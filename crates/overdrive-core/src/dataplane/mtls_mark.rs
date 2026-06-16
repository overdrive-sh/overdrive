//! `MTLS_LEG_S_DIAL_MARK` — the cross-adapter `SO_MARK` for the inbound
//! mTLS leg-S dial (F5 intercept-recursion exemption).
//!
//! A shared dataplane protocol constant on the same footing as
//! `maglev_table_size::DEFAULT_M` and `drop_class::DropClass`: the value
//! is read by BOTH adapters, so it has exactly one SSOT here in
//! `overdrive-core`. No I/O — a plain `pub const u32`.

/// The `SO_MARK` the agent stamps on its INBOUND leg-S dial (F5 inbound
/// intercept-recursion exemption).
///
/// The nft-TPROXY `prerouting` rule intercepts the server's virtual address;
/// the agent's leg-S dial targets that same logical address the client aimed
/// at, so without this mark the SYN would be TPROXY'd back to the agent's
/// leg-C listener, recursing instead of reaching the server. The production
/// nft-TPROXY rule excludes this mark; the test harness mirrors it.
///
/// Shared SSOT for both adapters: `overdrive-dataplane` STAMPS it (SO_MARK on
/// the leg-S dial, `mtls::dial_leg_s`); `overdrive-worker` EXCLUDES it (the
/// nft prerouting rule in `mtls_intercept::install_inbound_tproxy`).
pub const MTLS_LEG_S_DIAL_MARK: u32 = 0x2;
