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

/// The leg-C / leg-F TPROXY fwmark (DG-9). Today the worker's private
/// `TPROXY_FWMARK` and netlink's private `SHARED_IP_FWMARK`, both `0x1`; those
/// private copies are deleted when they consume this constant (DELIVER vsock
/// new-step 14).
pub const MTLS_TPROXY_MARK: u32 = 0x1;

/// The guest-flow owner's egress mark (D26-PINS F-6, DG-9). One dedicated bit:
/// the constant output rules match it under its mask (`meta mark & 0x4 ==
/// 0x4`). The owner's connect to leg-F stays unmarked.
pub const OWNER_EGRESS_MARK: u32 = 0x4;

/// The shared nft table (family `ip`; DG-9). Today private as `NFT_TABLE`
/// (worker) and `SHARED_IP_TABLE` (netlink).
pub const MTLS_SHARED_NFT_TABLE: &str = "overdrive-mtls";

#[cfg(test)]
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
mod tests {
    use super::{MTLS_LEG_S_DIAL_MARK, MTLS_SHARED_NFT_TABLE, MTLS_TPROXY_MARK, OWNER_EGRESS_MARK};

    /// DG-9: the three Overdrive marks are single bits and pairwise disjoint,
    /// so a rule matching one under its mask never matches another.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn overdrive_marks_are_single_bits_and_pairwise_disjoint() {
        let marks = [
            ("MTLS_TPROXY_MARK", MTLS_TPROXY_MARK),
            ("MTLS_LEG_S_DIAL_MARK", MTLS_LEG_S_DIAL_MARK),
            ("OWNER_EGRESS_MARK", OWNER_EGRESS_MARK),
        ];
        for (name, mark) in marks {
            assert_eq!(mark.count_ones(), 1, "{name} = {mark:#x} is one bit");
        }
        for (i, (a_name, a)) in marks.iter().enumerate() {
            for (b_name, b) in &marks[i + 1..] {
                assert_eq!(a & b, 0, "{a_name} and {b_name} overlap");
            }
        }
    }

    /// DG-9: the pinned values.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn overdrive_marks_and_table_carry_their_pinned_values() {
        assert_eq!(MTLS_TPROXY_MARK, 0x1);
        assert_eq!(MTLS_LEG_S_DIAL_MARK, 0x2);
        assert_eq!(OWNER_EGRESS_MARK, 0x4);
        assert_eq!(MTLS_SHARED_NFT_TABLE, "overdrive-mtls");
    }
}
