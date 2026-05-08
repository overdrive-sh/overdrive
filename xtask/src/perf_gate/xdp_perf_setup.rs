//! Pure iface-resolution + veth-pair argv construction for
//! `cargo xtask xdp-perf`.
//!
//! # Why this module exists
//!
//! Prior to this fix, the shell-side wrapper at `main.rs::xdp_perf`
//! defaulted `OVERDRIVE_XDP_PERF_IFACE` to `"lo"` (with a doc claim
//! that "`lo` is the safe default for a local sanity-run"). The
//! claim is wrong: the Linux loopback driver does not implement
//! `ndo_bpf` for native XDP, so `xdp-bench drop lo` exits 4 from
//! libbpf with `Underlying driver does not support XDP in native
//! mode`. Every uncustomised `cargo xtask xdp-perf` invocation —
//! local Lima, the GitHub Actions `xdp-perf` job, the Tier 4 gate
//! per `.claude/rules/testing.md` — failed the same way.
//!
//! The fix replaces the broken default with an auto-provisioned
//! `xdp0/xdp1` veth pair: `xdp0` is the test-target interface that
//! `xdp-bench` attaches to, `xdp1` is the peer side that completes
//! the link so the kernel brings both UP. Veth devices implement
//! the full XDP feature set including native attach (per
//! `drivers/net/veth.c::veth_xdp` and `ndo_bpf = veth_xdp`), so the
//! attach succeeds without `--xdp-mode skb` fallback.
//!
//! # Pure / impure split
//!
//! This module owns the *decisions* (which iface name, which
//! provisioning plan, which argv shape). The *I/O* — running
//! `ip link show` to detect existence, running `ip link add` to
//! create the pair, running `ip link set up` to bring them up —
//! lives in `main.rs::xdp_perf`. Decisions are unit-tested at
//! `xtask/tests/perf_gate_self_test.rs`; the runtime invocation
//! is exercised by every Lima / CI run of the gate itself.
//!
//! # Naming
//!
//! `xdp0` / `xdp1` are project-reserved interface names. Changing
//! them is a breaking operational change — every host that has
//! ever run the gate will accumulate stale veth pairs under the
//! old names until something explicitly removes them. The
//! `default_veth_names_are_stable` self-test pins the contract.

/// Project-reserved primary interface name for the auto-provisioned
/// XDP-perf veth pair. The XDP program attaches here.
pub const DEFAULT_VETH_PRIMARY: &str = "xdp0";

/// Project-reserved peer interface name.
///
/// Required for the link to come UP — XDP-target attach against
/// `xdp0` requires its peer (`xdp1`) to also be present and UP,
/// otherwise the link state stays `LOWER_DOWN` and packet flow
/// doesn't establish.
pub const DEFAULT_VETH_PEER: &str = "xdp1";

/// Resolved iface configuration handed back to the shell-side
/// wrapper. Carries both the iface name `xdp-bench` will attach to
/// and the provisioning plan the wrapper executes before the
/// invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IfaceResolution {
    /// Interface name passed to `xdp-bench` as the positional `<ifname>`
    /// argument. Always equal to `provisioning.primary` in the
    /// auto-provision case (cross-field invariant pinned by
    /// `auto_provision_primary_matches_resolved_iface`).
    pub iface: String,

    /// What the wrapper does before invoking `xdp-bench`.
    pub provisioning: ProvisioningPlan,
}

/// What the shell-side wrapper does to `/sys/class/net` before
/// calling `xdp-bench`.
///
/// Two variants — the resolver never returns a third. Adding one is
/// a deliberate API change that needs a self-test update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvisioningPlan {
    /// Idempotently create a veth pair `(primary, peer)` and bring
    /// both sides UP. The wrapper checks `ip link show <primary>`
    /// first; on exit-zero (already exists), it skips the create
    /// step but still runs the `set up` calls (cheap, idempotent).
    AutoProvisionVeth { primary: String, peer: String },

    /// Use the resolved iface as-is — its lifecycle is the
    /// caller's. Triggered by an explicit `OVERDRIVE_XDP_PERF_IFACE`
    /// env var or by the `--no-auto-setup` CLI flag.
    UseAsIs,
}

/// Pure resolution: turns environment + CLI flags into a concrete
/// `IfaceResolution`.
///
/// Decision matrix (`env_iface` × `no_auto_setup`):
///
/// | `env_iface` | `no_auto_setup` | iface            | provisioning           |
/// |-------------|-----------------|------------------|------------------------|
/// | `Some(s)`   | `false`         | `s`              | `UseAsIs`              |
/// | `Some(s)`   | `true`          | `s`              | `UseAsIs`              |
/// | `None`      | `false`         | `xdp0` (default) | `AutoProvisionVeth`    |
/// | `None`      | `true`          | `xdp0` (default) | `UseAsIs`              |
///
/// The bottom-right cell — env unset, `no_auto_setup=true` — produces
/// an attach attempt against an interface the wrapper has not
/// staged. That's an explicit operator choice (typically a harness
/// that pre-stages `xdp0` itself), and the wrapper trusts it. If the
/// interface is missing the libbpf attach will surface the failure
/// at `xdp-bench` time with the same error class the operator would
/// see running `xdp-bench` directly.
#[must_use]
pub fn resolve_iface_config(env_iface: Option<String>, no_auto_setup: bool) -> IfaceResolution {
    match env_iface {
        Some(iface) => IfaceResolution { iface, provisioning: ProvisioningPlan::UseAsIs },
        None if no_auto_setup => IfaceResolution {
            iface: DEFAULT_VETH_PRIMARY.to_string(),
            provisioning: ProvisioningPlan::UseAsIs,
        },
        None => {
            let primary = DEFAULT_VETH_PRIMARY.to_string();
            IfaceResolution {
                iface: primary.clone(),
                provisioning: ProvisioningPlan::AutoProvisionVeth {
                    primary,
                    peer: DEFAULT_VETH_PEER.to_string(),
                },
            }
        }
    }
}

/// `ip link add <primary> type veth peer name <peer>` — argv
/// vector. Caller wraps in `Command::new("ip").args(&argv[1..])`.
///
/// The `argv[0] == "ip"` convention keeps the test assertion
/// readable as a single literal vector and lets a future migration
/// to `nsenter` / busybox-style invocation add a prefix without
/// reshuffling the rest of the call.
#[must_use]
pub fn veth_create_argv(primary: &str, peer: &str) -> Vec<String> {
    vec![
        "ip".to_string(),
        "link".to_string(),
        "add".to_string(),
        primary.to_string(),
        "type".to_string(),
        "veth".to_string(),
        "peer".to_string(),
        "name".to_string(),
        peer.to_string(),
    ]
}

/// `ip link set <iface> up` — argv vector.
///
/// Run once per side of the veth pair. Bringing only one side UP
/// leaves the link `LOWER_DOWN`; the kernel still allows attach but
/// no packets flow, which would silently break `xdp-bench`'s
/// drop-rate measurement.
#[must_use]
pub fn veth_link_up_argv(iface: &str) -> Vec<String> {
    vec![
        "ip".to_string(),
        "link".to_string(),
        "set".to_string(),
        iface.to_string(),
        "up".to_string(),
    ]
}

/// `ip link show <iface>` — existence-check argv.
///
/// The wrapper uses the exit status (0 = exists, non-zero = absent)
/// to decide whether to skip creation. Output is discarded — no
/// `--json` flag or other format-version-sensitive coupling.
#[must_use]
pub fn veth_show_argv(iface: &str) -> Vec<String> {
    vec!["ip".to_string(), "link".to_string(), "show".to_string(), iface.to_string()]
}
