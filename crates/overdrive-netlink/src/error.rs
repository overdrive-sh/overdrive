//! The shared, errno-carrying netlink error type (ADR-0085 D3).
//!
//! [`NetlinkError`] is the low-level error every `overdrive-netlink`
//! operation returns. It is **embedded** (via `#[source]`) into the
//! consuming crates' per-call-site error enums (`VethProvisionError`,
//! `InterceptError`) so the operator keeps a cause-specific, per-step
//! message — the shared error is embedded, never substituted for the
//! per-site variants (`.claude/rules/development.md` § Errors).
//!
//! The single load-bearing accessor is [`NetlinkError::errno`]: it returns
//! the **typed** kernel errno (a negative code, e.g. `-EEXIST` / `-ENODEV`)
//! that the converge executors match on to swallow idempotent-success
//! conditions — the typed replacement for the old
//! `stderr.contains("File exists")` / `link_absent` substring matching that
//! could silently reclassify a genuine failure on the packet-corruption
//! path.

use std::num::NonZeroI32;

/// `-EEXIST` (address / kernel-auto on-link route already present). Netlink
/// reports errors as a **negative** errno (`netlink(7)`), so the idempotent
/// codes are negative.
pub const NEG_EEXIST: i32 = -libc::EEXIST;
/// `-ENODEV` (the link is absent — on a del, or on an observe read).
pub const NEG_ENODEV: i32 = -libc::ENODEV;

/// The low-level, errno-carrying netlink error.
///
/// Variants are keyed by the netlink op family (link / address / route over
/// `NETLINK_ROUTE`; the hand-rolled ethtool genl over `NETLINK_GENERIC`) and
/// each carries the typed kernel errno via [`NetlinkError::errno`] — never a
/// parsed stderr string.
#[derive(Debug, thiserror::Error)]
pub enum NetlinkError {
    /// Opening the netlink socket / spawning the rtnetlink connection failed
    /// (the discrete replacement for the obsolete blanket `Spawn(#[from]
    /// io::Error)` — ADR-0085 D3). Carries no kernel errno.
    #[error("opening a netlink socket failed: {source}")]
    Connect {
        /// The underlying socket-open failure.
        source: std::io::Error,
    },
    /// An `rtnetlink` link operation (`add` / `del` / `get` / `set up`)
    /// returned a kernel `NLMSG_ERROR`.
    #[error("netlink link {op} failed: {source}")]
    Link {
        /// The failing op (`add` / `del` / `set-up` / `get`).
        op: &'static str,
        /// The rtnetlink error — its `Display` renders the human errno
        /// (`Operation not permitted (os error 1)`), which the Tier-3
        /// unprivileged-runner skip detection greps for. Boxed because
        /// `rtnetlink::Error::UnexpectedMessage` embeds a full netlink
        /// message (~160 bytes), which would otherwise inflate every
        /// `Result<_, VethProvisionError>` past the `result_large_err` bound.
        source: Box<rtnetlink::Error>,
    },
    /// An `rtnetlink` address operation (`add`) returned a kernel
    /// `NLMSG_ERROR`.
    #[error("netlink address {op} failed: {source}")]
    Address {
        /// The failing op (`add`).
        op: &'static str,
        /// The rtnetlink error (`Display` renders the human errno). Boxed —
        /// see [`NetlinkError::Link`].
        source: Box<rtnetlink::Error>,
    },
    /// An `rtnetlink` route operation (`add`) returned a kernel
    /// `NLMSG_ERROR`.
    #[error("netlink route {op} failed: {source}")]
    Route {
        /// The failing op (`add`).
        op: &'static str,
        /// The rtnetlink error (`Display` renders the human errno). Boxed —
        /// see [`NetlinkError::Link`].
        source: Box<rtnetlink::Error>,
    },
    /// The named link was absent when an operation that requires it (address
    /// add, set-up, route add) tried to resolve its ifindex. Carries a
    /// synthetic `-ENODEV` so the converge executor swallows it as the
    /// idempotent "already gone; the next converge recreates" condition —
    /// the same class as an absent-on-observe read.
    #[error("netlink target link `{iface}` is absent")]
    LinkAbsent {
        /// The link name that could not be resolved.
        iface: String,
    },
    /// The hand-rolled ethtool `FEATURES_GET` / `FEATURES_SET` genl op failed
    /// — either a socket I/O error or a kernel `NLMSG_ERROR` code (rendered
    /// as an `io::Error` via `from_raw_os_error(|errno|)`).
    #[error("ethtool {op} failed: {source}")]
    Ethtool {
        /// The failing op (`resolve-family` / `features-get` / `features-set`).
        op: &'static str,
        /// The socket / kernel-errno failure. `raw_os_error()` is the
        /// positive errno; [`NetlinkError::errno`] re-negates it to the
        /// netlink `-errno` convention.
        source: std::io::Error,
    },
    /// Entering a network namespace failed — opening `/var/run/netns/<name>`
    /// or `setns(CLONE_NEWNET)` (the `in_netns` dedicated-thread helper, D4).
    #[error("entering netns `{netns}` failed: {source}")]
    Setns {
        /// The target netns name.
        netns: String,
        /// The `open`/`setns` failure.
        source: std::io::Error,
    },
    /// **Transitional bridge (removed in slice 01-02).** Carries a legacy
    /// `ip`/`ethtool`/`sysctl` subprocess failure (`stderr` + exit `status`)
    /// so the still-subprocess **per-allocation** helpers can populate the
    /// `#[source] NetlinkError` field on the shared per-site error variants
    /// while their netlink swap is pending. The host-netns path (this slice's
    /// deliverable) NEVER constructs this — its errors are typed
    /// [`Self::Link`] / [`Self::Address`] / [`Self::Route`] / [`Self::Ethtool`]
    /// with a real kernel errno. This variant, and its callers, are deleted
    /// when slice 01-02 swaps the per-alloc executor/observer to netlink
    /// (ADR-0085 D3/D10; the per-alloc path is 01-02 scope).
    #[error("{stderr}")]
    Subprocess {
        /// The captured subprocess stderr (trimmed).
        stderr: String,
        /// The subprocess exit status code (NOT a kernel errno).
        status: Option<i32>,
    },
}

impl NetlinkError {
    /// Construct a [`NetlinkError::Connect`].
    #[must_use]
    pub const fn connect(source: std::io::Error) -> Self {
        Self::Connect { source }
    }

    /// Construct a [`NetlinkError::Link`].
    #[must_use]
    pub fn link(op: &'static str, source: rtnetlink::Error) -> Self {
        Self::Link { op, source: Box::new(source) }
    }

    /// Construct a [`NetlinkError::Address`].
    #[must_use]
    pub fn address(op: &'static str, source: rtnetlink::Error) -> Self {
        Self::Address { op, source: Box::new(source) }
    }

    /// Construct a [`NetlinkError::Route`].
    #[must_use]
    pub fn route(op: &'static str, source: rtnetlink::Error) -> Self {
        Self::Route { op, source: Box::new(source) }
    }

    /// Construct a [`NetlinkError::LinkAbsent`].
    #[must_use]
    pub fn link_absent(iface: impl Into<String>) -> Self {
        Self::LinkAbsent { iface: iface.into() }
    }

    /// Construct a [`NetlinkError::Ethtool`].
    #[must_use]
    pub const fn ethtool(op: &'static str, source: std::io::Error) -> Self {
        Self::Ethtool { op, source }
    }

    /// Construct a [`NetlinkError::Setns`].
    #[must_use]
    pub fn setns(netns: impl Into<String>, source: std::io::Error) -> Self {
        Self::Setns { netns: netns.into(), source }
    }

    /// Construct the transitional [`NetlinkError::Subprocess`] bridge (per-alloc
    /// subprocess helpers only; removed in slice 01-02).
    #[must_use]
    pub fn subprocess(stderr: impl Into<String>, status: Option<i32>) -> Self {
        Self::Subprocess { stderr: stderr.into(), status }
    }

    /// The typed kernel errno, as the **negative** code netlink reports
    /// (`-EEXIST` = `-17`, `-ENODEV` = `-19`), or `None` for a structural /
    /// non-errno failure (a decode error, a connect failure). This is the
    /// accessor the converge executors' idempotency `match` reads — ADR-0085
    /// D3 pins this signature; it is not a field access the crafter
    /// improvises.
    #[must_use]
    pub fn errno(&self) -> Option<i32> {
        match self {
            Self::Link { source, .. }
            | Self::Address { source, .. }
            | Self::Route { source, .. } => rtnetlink_errno(source),
            Self::LinkAbsent { .. } => Some(NEG_ENODEV),
            // The ethtool / setns `io::Error` carries the POSITIVE errno;
            // re-negate to the netlink `-errno` convention so a caller sees
            // one sign.
            Self::Ethtool { source, .. } | Self::Setns { source, .. } => {
                source.raw_os_error().map(|raw| -raw.abs())
            }
            // A subprocess exit code is NOT a kernel errno — the transitional
            // per-alloc bridge carries no idempotency signal (its callers
            // already swallowed the benign cases via stderr substrings; 01-02
            // replaces this with a typed errno).
            Self::Connect { .. } | Self::Subprocess { .. } => None,
        }
    }
}

/// Extract the typed (negative) errno from an `rtnetlink::Error`. Only a
/// kernel `NLMSG_ERROR` (`Error::NetlinkError`) carries one; every other
/// variant is a structural/decode failure with no errno.
fn rtnetlink_errno(err: &rtnetlink::Error) -> Option<i32> {
    match err {
        rtnetlink::Error::NetlinkError(msg) => msg.code.map(NonZeroI32::get),
        _ => None,
    }
}

/// True when `errno` denotes an **idempotent-success** condition the converge
/// executors swallow rather than surfacing.
///
/// The swallowed codes are `-EEXIST` (address / kernel-auto on-link route
/// already present) and `-ENODEV` (the link is absent — on a del or an
/// observe). `None` (a structural / non-errno failure) and every other code
/// are FATAL and surface. This is the typed replacement for the old
/// `stderr.contains("File exists")` / `link_absent` substring classification
/// (ADR-0085 D3) — a locale/version phrase can drift; a negative errno
/// cannot.
#[must_use]
pub const fn errno_is_idempotent(errno: Option<i32>) -> bool {
    matches!(errno, Some(code) if code == NEG_EEXIST || code == NEG_ENODEV)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // The two idempotent codes are swallowed; `None` and every other code
    // are fatal. Property over the full `i32` errno space (ADR-0085 D3
    // classifier arms) — the benign/fatal partition is exactly
    // `{-EEXIST, -ENODEV}`.
    proptest! {
        #[test]
        fn errno_classifier_swallows_exactly_eexist_and_enodev(code in any::<i32>()) {
            let benign = code == NEG_EEXIST || code == NEG_ENODEV;
            prop_assert_eq!(errno_is_idempotent(Some(code)), benign);
        }
    }

    /// `None` (a structural / non-errno failure — a decode error, a connect
    /// failure) is NEVER idempotent: it must surface, not be swallowed.
    #[test]
    fn none_errno_is_fatal_never_swallowed() {
        assert!(!errno_is_idempotent(None));
    }

    /// The two named idempotent codes are the negative errno convention
    /// netlink uses — pin them so a future refactor cannot silently flip the
    /// sign (a positive `EEXIST` would never match a netlink `-EEXIST`).
    #[test]
    fn idempotent_codes_are_the_negative_errno_convention() {
        assert_eq!(NEG_EEXIST, -17);
        assert_eq!(NEG_ENODEV, -19);
        assert!(errno_is_idempotent(Some(NEG_EEXIST)));
        assert!(errno_is_idempotent(Some(NEG_ENODEV)));
        // The positive forms are NOT the netlink convention and must NOT
        // classify as benign.
        assert!(!errno_is_idempotent(Some(libc::EEXIST)));
        assert!(!errno_is_idempotent(Some(libc::ENODEV)));
    }

    /// A `LinkAbsent` error reports `-ENODEV` through the typed accessor, so
    /// the executor swallows it as the idempotent "already gone" condition.
    #[test]
    fn link_absent_reports_enodev_and_is_idempotent() {
        let err = NetlinkError::link_absent("ovd-veth-cli");
        assert_eq!(err.errno(), Some(NEG_ENODEV));
        assert!(errno_is_idempotent(err.errno()));
    }

    /// A connect failure carries no kernel errno and is fatal.
    #[test]
    fn connect_error_has_no_errno_and_is_fatal() {
        let err = NetlinkError::connect(std::io::Error::from_raw_os_error(libc::EACCES));
        assert_eq!(err.errno(), None);
        assert!(!errno_is_idempotent(err.errno()));
    }
}
