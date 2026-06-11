//! Workload-SVID validity-window constants — the SINGLE source of truth for
//! the leaf TTL and the clock-skew back-off, shared by the **issuer** (the
//! `overdrive-host` `RcgenCa` adapter that signs the leaf) and the **auditor**
//! (the `overdrive-control-plane` `ca_issuance` seam that records the
//! `issued_certificates` audit window, ADR-0063 D6).
//!
//! Both crates depend on `overdrive-core`, which is the only crate they share;
//! co-locating the constants here is the only way the validity window the host
//! adapter SIGNS and the audit window the control plane RECORDS cannot drift.
//! Defining a matching constant in each crate (the rejected alternative)
//! reintroduces exactly the drift this module exists to prevent: a later edit
//! to one copy silently leaves the other recording a window the platform never
//! issued for.
//!
//! These are pure values (`const Duration`); `overdrive-core` is class `core`
//! and dst-lint-clean — a `const Duration` is a value, not a clock read, the
//! same shape as the [`DEFAULT_STARTUP_DEADLINE`](crate::service_lifecycle)
//! and [`RESTART_BACKOFF_DURATION`](crate::reconcilers) constants already
//! living in this crate.

use std::time::Duration;

/// SVID leaf validity width — ~1 hour (research Finding 6 / ADR-0063). Short-
/// lived workload identities keep the node-compromise / rotation blast radius
/// small; the #40 near-expiry reissue action (`SvidLifecycle`'s
/// `rotate-svid`-correlated `Action::IssueSvid`) re-issues before expiry.
///
/// The issuer sets `not_after = not_before + WORKLOAD_SVID_TTL`; the audit row
/// records the SAME width. There is no separate "audit TTL" — the window the
/// audit row records IS, by definition, the window the leaf was issued for.
pub const WORKLOAD_SVID_TTL: Duration = Duration::from_secs(3600);

/// Clock-skew back-off applied to the SVID `not_before`. A freshly-minted leaf
/// must verify under a relying party whose clock is marginally behind the
/// issuer's; backing `not_before` off by this margin avoids a spurious
/// "certificate is not yet valid" rejection at the verify boundary.
///
/// The issuer signs `not_before = now − SKEW_TOLERANCE`; the auditor records
/// the SAME back-off so the recorded `not_before` is faithful to the window
/// the platform actually issued for, not 60 s later than it.
pub const SKEW_TOLERANCE: Duration = Duration::from_secs(60);
