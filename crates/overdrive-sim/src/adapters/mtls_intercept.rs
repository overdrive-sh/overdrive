//! `SimMtlsIntercept` — in-memory
//! [`MtlsIntercept`](overdrive_worker::mtls_intercept_port::MtlsIntercept)
//! double with per-method fault scripting (GH #250, ADR-0076 § 4.6).
//!
//! The sim counterpart to `overdrive_worker::mtls_intercept_port::HostMtlsIntercept`.
//! It exists for ONE reason: nothing in the tree can make
//! `MtlsInterceptWorker::start_alloc` fail on demand, so the security-relevant
//! call-site ordering the action-shim's fail-closed arms depend on (a
//! now-`Failed` allocation must never release its exit watcher) is otherwise
//! wholly untested. This double is the mechanism that makes it exercisable.
//!
//! # What it models, and what it does not
//!
//! - **Fault arms are PURE** — an armed fault short-circuits before any
//!   syscall, so a test that drives only fault arms performs ZERO I/O and
//!   belongs in the DEFAULT lane.
//! - **The `Ok` arm of `bind_transparent` binds a REAL, PLAIN
//!   (non-`IP_TRANSPARENT`) loopback listener** (DFS-5). The worker's `Ok` path
//!   consumes a live listener it accepts on, and there is no way to fabricate a
//!   [`std::net::TcpListener`] without a syscall. **Any test that drives this
//!   `Ok` arm binds a socket and is therefore INTEGRATION-lane** per
//!   `.claude/rules/testing.md` § "Integration vs unit gating".
//! - **The `Ok` arm of the two installs returns an INERT guard** that records
//!   nothing and whose `Drop` is a no-op. The double installs no nft rule, so
//!   there is none to remove.
//!
//! # Determinism
//!
//! Holds no clock, no entropy, no store, and no collection whose iteration
//! order is observed. Each method's outcome is a pure function of its armed
//! fault, and the three slots carry no cross-slot invariant.

use std::net::SocketAddrV4;

use overdrive_worker::mtls_intercept::{InterceptError, Result};
use overdrive_worker::mtls_intercept_port::{InterceptGuard, MtlsIntercept};
use parking_lot::Mutex;

/// A scripted intercept-install fault, expressed in the REAL error shapes the
/// production substrate produces (research Finding 5.3 — inject errors that
/// naturally occur, not a generic boolean "fail now").
///
/// `Clone` because a scripted fault is STANDING, not one-shot (see
/// [`SimMtlsIntercept`]): the same fault must be re-materialised on every call
/// while armed, and [`InterceptError`] is not `Clone` (it carries
/// [`std::io::Error`]).
#[derive(Debug, Clone)]
pub enum SimInterceptFault {
    /// Materialises `InterceptError::TransparentListener { addr, source:
    /// io::Error::from_raw_os_error(errno) }` — the missing-`CAP_NET_ADMIN`
    /// (`libc::EPERM`), unsupported-setopt (`libc::ENOPROTOOPT`), and
    /// address-in-use (`libc::EADDRINUSE`) shapes. `addr` is the address the
    /// faulted call was made with.
    TransparentListener {
        /// The `errno` the real syscall would have returned.
        errno: i32,
    },
    /// Materialises `InterceptError::TproxyInstall { reason }` — the
    /// `nft`-exited-non-zero / `nft`-binary-missing / `ip rule` shape.
    TproxyInstall {
        /// The failing-command description the real adapter would report.
        reason: String,
    },
}

/// In-memory [`MtlsIntercept`] double with per-method fault scripting.
///
/// # Fault lifetime — STANDING, not one-shot
///
/// An armed fault fires on EVERY subsequent call to that method until re-armed
/// or [`clear_faults`](Self::clear_faults). This deliberately diverges from
/// `SimMtlsResolve`'s consume-on-use `.take()` shape, because the faults differ
/// in kind: a poisoned store handle is transient, whereas a missing
/// `CAP_NET_ADMIN` or an absent `nft` binary fails EVERY call. Standing faults
/// also remove call-order dependence — `start_alloc` calls `bind_transparent`
/// twice (leg-F then leg-C), and a consume-on-use fault would make "which leg
/// failed" an artifact of ordering rather than the test's explicit choice.
///
/// # Out-of-contract fault pairings
///
/// [`SimInterceptFault`] is one type shared by all three scripting helpers, so
/// the compiler permits arming a
/// [`TproxyInstall`](SimInterceptFault::TproxyInstall) fault on
/// [`bind_transparent`](MtlsIntercept::bind_transparent) (or a
/// [`TransparentListener`](SimInterceptFault::TransparentListener) fault on an
/// install). Doing so produces an [`InterceptError`] variant that method's
/// contract says it never returns, so the double would model a substrate the
/// real one cannot exhibit. The SANCTIONED pairings are `bind_transparent` ⇔
/// `TransparentListener` and `install_*` ⇔ either variant (the `Inbound` arm
/// legitimately carries both, per `MtlsInterceptInstallError::stage`). Arming
/// any other pairing is a test defect, not a supported scenario.
#[expect(
    clippy::struct_field_names,
    reason = "the shared `_fault` postfix is load-bearing: each field is the STANDING fault slot \
              backing one trait method, and the prefix names that method. Dropping it would leave \
              `bind` / `outbound` / `inbound`, which read as the installs themselves rather than \
              as the faults armed against them. Names pinned verbatim by ADR-0076 § 4.6."
)]
pub struct SimMtlsIntercept {
    /// Standing fault for [`bind_transparent`](MtlsIntercept::bind_transparent).
    bind_fault: Mutex<Option<SimInterceptFault>>,
    /// Standing fault for [`install_outbound`](MtlsIntercept::install_outbound).
    outbound_fault: Mutex<Option<SimInterceptFault>>,
    /// Standing fault for [`install_inbound`](MtlsIntercept::install_inbound).
    inbound_fault: Mutex<Option<SimInterceptFault>>,
}

/// The inert guard the double's install `Ok` arms return. No rule was
/// installed, so `Drop` removes nothing. Private — consumers see only
/// `Box<dyn InterceptGuard>`.
struct InertGuard;

impl InterceptGuard for InertGuard {}

impl SimMtlsIntercept {
    /// A double with NO fault armed — every method takes its `Ok` arm. Faults
    /// are armed explicitly through the scripting helpers below (no builder
    /// over a *dependency*; these script OUTCOMES, mirroring `SimMtlsResolve`).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bind_fault: Mutex::new(None),
            outbound_fault: Mutex::new(None),
            inbound_fault: Mutex::new(None),
        }
    }

    /// Arm a STANDING fault on `bind_transparent`. Fires on every subsequent
    /// call until re-armed or [`clear_faults`](Self::clear_faults).
    pub fn script_bind_fault(&self, fault: SimInterceptFault) {
        *self.bind_fault.lock() = Some(fault);
    }

    /// Arm a STANDING fault on `install_outbound`.
    pub fn script_outbound_fault(&self, fault: SimInterceptFault) {
        *self.outbound_fault.lock() = Some(fault);
    }

    /// Arm a STANDING fault on `install_inbound`.
    pub fn script_inbound_fault(&self, fault: SimInterceptFault) {
        *self.inbound_fault.lock() = Some(fault);
    }

    /// Disarm every standing fault.
    pub fn clear_faults(&self) {
        *self.bind_fault.lock() = None;
        *self.outbound_fault.lock() = None;
        *self.inbound_fault.lock() = None;
    }
}

impl Default for SimMtlsIntercept {
    fn default() -> Self {
        Self::new()
    }
}

/// Materialise an armed fault descriptor into a FRESH [`InterceptError`].
///
/// A fresh error per call is what makes the STANDING lifetime possible:
/// [`InterceptError`] is not `Clone` (it carries [`std::io::Error`]), so the
/// slot holds the re-materialisable descriptor and this function rebuilds the
/// error on every fire. `addr` is the address the faulted call was made with —
/// it is carried only by the `TransparentListener` shape.
fn materialise(fault: SimInterceptFault, addr: SocketAddrV4) -> InterceptError {
    match fault {
        SimInterceptFault::TransparentListener { errno } => InterceptError::TransparentListener {
            addr,
            source: std::io::Error::from_raw_os_error(errno),
        },
        SimInterceptFault::TproxyInstall { reason } => InterceptError::TproxyInstall { reason },
    }
}

/// Read the armed fault out of `slot` WITHOUT consuming it (DFS-4 — the fault
/// is standing, so it must survive the call that fires it). Cloning the
/// descriptor rather than `.take()`ing it is the whole mechanism: a `.take()`
/// here would let the second call fall through to the `Ok` arm.
///
/// The clone lands in a local before the caller branches, so the lock guard is
/// released at the end of this function rather than held across the branch.
fn armed(slot: &Mutex<Option<SimInterceptFault>>) -> Option<SimInterceptFault> {
    slot.lock().clone()
}

impl MtlsIntercept for SimMtlsIntercept {
    fn bind_transparent(&self, addr: SocketAddrV4) -> Result<std::net::TcpListener> {
        // The fault arm is PURE — it short-circuits BEFORE the bind below, so a
        // test driving only this arm performs zero I/O and stays default-lane.
        if let Some(fault) = armed(&self.bind_fault) {
            return Err(materialise(fault, addr));
        }

        // DFS-5: the `Ok` arm binds a REAL, PLAIN (non-`IP_TRANSPARENT`)
        // listener. There is no way to fabricate a `TcpListener` without a
        // syscall, and the worker's `Ok` path consumes a live listener it
        // accepts on. Any test reaching here is INTEGRATION-lane.
        std::net::TcpListener::bind(addr)
            .map_err(|source| InterceptError::TransparentListener { addr, source })
    }

    fn install_outbound(
        &self,
        _host_veth: &str,
        _agent_leg_f_port: u16,
    ) -> Result<Box<dyn InterceptGuard>> {
        if let Some(fault) = armed(&self.outbound_fault) {
            // No address is in scope on this method, so a `TransparentListener`
            // descriptor armed here — an OUT-OF-CONTRACT pairing (see the type
            // docs: a test defect, not a supported scenario) — materialises
            // against the loopback wildcard. The sanctioned outbound pairing is
            // `TproxyInstall`, which carries no address at all.
            return Err(materialise(fault, SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0)));
        }

        // The double installs no nft rule, so the guard owns nothing and its
        // `Drop` releases nothing — honouring the `InterceptGuard` contract.
        Ok(Box::new(InertGuard))
    }

    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        _agent_leg_c_port: u16,
    ) -> Result<Box<dyn InterceptGuard>> {
        if let Some(fault) = armed(&self.inbound_fault) {
            return Err(materialise(fault, virt));
        }

        Ok(Box::new(InertGuard))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    /// The canonical leg-F / leg-C bind address the production caller uses.
    const LEG_ADDR: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0);

    /// A declared Service listener address — the `virt` shape `install_inbound`
    /// is called with.
    const VIRT: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 8080);

    /// Which trait method a S-MIF-06 case drives.
    #[derive(Debug, Clone, Copy)]
    enum Method {
        BindTransparent,
        InstallOutbound,
        InstallInbound,
    }

    /// The `Err` payload a S-MIF-06 case expects, in the shape the REAL
    /// substrate reports it.
    #[derive(Debug, Clone, Copy)]
    enum ExpectedErr {
        /// `InterceptError::TransparentListener` whose `source.raw_os_error()`
        /// is exactly this `errno`.
        TransparentListener(i32),
        /// `InterceptError::TproxyInstall` carrying exactly this `reason`.
        TproxyInstall(&'static str),
    }

    /// The 4 SANCTIONED S-MIF-06 pairings — `(method, armed fault, expected
    /// error)` — exhausting the two variants [`SimInterceptFault`] can
    /// materialise. The UNSANCTIONED pairings (a `TproxyInstall` fault on
    /// `bind_transparent`, a `TransparentListener` fault on `install_outbound`)
    /// model a substrate the real one cannot exhibit and are a documented test
    /// defect, so they are deliberately absent.
    fn sanctioned_pairings() -> [(Method, SimInterceptFault, ExpectedErr); 4] {
        const NFT_MISSING: &str = "nft: command not found";
        const NFT_LOCKED: &str = "nft add rule exited 1: ruleset lock contention";

        [
            (
                Method::BindTransparent,
                SimInterceptFault::TransparentListener { errno: libc::EPERM },
                ExpectedErr::TransparentListener(libc::EPERM),
            ),
            (
                Method::InstallOutbound,
                SimInterceptFault::TproxyInstall { reason: NFT_MISSING.to_owned() },
                ExpectedErr::TproxyInstall(NFT_MISSING),
            ),
            (
                Method::InstallInbound,
                SimInterceptFault::TproxyInstall { reason: NFT_LOCKED.to_owned() },
                ExpectedErr::TproxyInstall(NFT_LOCKED),
            ),
            (
                Method::InstallInbound,
                SimInterceptFault::TransparentListener { errno: libc::ENOPROTOOPT },
                ExpectedErr::TransparentListener(libc::ENOPROTOOPT),
            ),
        ]
    }

    /// Drive `method` on `sut` and return the `Err` it must produce. Every
    /// caller here has a fault armed, so an `Ok` is a test failure — and for
    /// `bind_transparent` an `Ok` would additionally bind a real socket, which
    /// is exactly the default-lane I/O this suite must not perform.
    fn drive_expecting_err(sut: &SimMtlsIntercept, method: Method) -> InterceptError {
        match method {
            Method::BindTransparent => sut
                .bind_transparent(LEG_ADDR)
                .expect_err("an armed bind fault short-circuits before any syscall"),
            // `Box<dyn InterceptGuard>` is not `Debug` (the guard's entire
            // contract is its `Drop`), so the `Ok` payload is mapped away
            // before `expect_err`.
            Method::InstallOutbound => sut
                .install_outbound("veth-alloc0", 4001)
                .map(|_guard| ())
                .expect_err("an armed outbound fault short-circuits"),
            Method::InstallInbound => sut
                .install_inbound(VIRT, 4002)
                .map(|_guard| ())
                .expect_err("an armed inbound fault short-circuits"),
        }
    }

    /// Arm `fault` on the slot backing `method`.
    fn arm(sut: &SimMtlsIntercept, method: Method, fault: SimInterceptFault) {
        match method {
            Method::BindTransparent => sut.script_bind_fault(fault),
            Method::InstallOutbound => sut.script_outbound_fault(fault),
            Method::InstallInbound => sut.script_inbound_fault(fault),
        }
    }

    /// Assert `got` is exactly the [`ExpectedErr`] shape — the `Err`
    /// discriminant AND its payload.
    fn assert_err_shape(got: &InterceptError, expected: ExpectedErr) {
        match expected {
            ExpectedErr::TransparentListener(errno) => match got {
                InterceptError::TransparentListener { source, .. } => assert_eq!(
                    source.raw_os_error(),
                    Some(errno),
                    "TransparentListener must carry the armed errno",
                ),
                other => panic!("expected TransparentListener, got {other:?}"),
            },
            ExpectedErr::TproxyInstall(reason) => match got {
                InterceptError::TproxyInstall { reason: got_reason } => {
                    assert_eq!(got_reason, reason, "TproxyInstall must carry the armed reason");
                }
                other => panic!("expected TproxyInstall, got {other:?}"),
            },
        }
    }

    /// S-MIF-06 — an armed fault surfaces as exactly the error the REAL
    /// substrate produces, across the 4 sanctioned pairings.
    ///
    /// Universe (port-exposed): the `Result` the trait method returns — its
    /// `Err` discriminant, plus `source.raw_os_error()` for
    /// `TransparentListener` and `reason` for `TproxyInstall`. Nothing reads a
    /// private slot; the scripting helpers are the only writes and the trait
    /// method the only read.
    ///
    /// Realism criterion (research Finding 5.3, DFS-4): the faults are armed in
    /// the REAL shapes the substrate produces — `libc::EPERM` is the
    /// missing-`CAP_NET_ADMIN` shape, `libc::ENOPROTOOPT` the
    /// kernel-without-`IP_TRANSPARENT` shape — never a generic "fail now".
    #[test]
    fn armed_fault_surfaces_as_the_real_substrate_error() {
        for (method, fault, expected) in sanctioned_pairings() {
            let sut = SimMtlsIntercept::new();
            arm(&sut, method, fault);

            let got = drive_expecting_err(&sut, method);

            assert_err_shape(&got, expected);
        }
    }

    /// S-MIF-07 — an armed fault is STANDING (DFS-4): it fires on BOTH of two
    /// consecutive calls with the same cause, and does not decay.
    ///
    /// Universe: the two `Result`s from the two consecutive `bind_transparent`
    /// calls — both `Err`, both `TransparentListener`, both carrying the armed
    /// `raw_os_error()`.
    ///
    /// Two calls, not `n`: two is the minimum that distinguishes standing from
    /// one-shot, and it is the exact cardinality the production caller exhibits
    /// (`start_alloc` binds leg-F then leg-C). A `.take()`-instead-of-clone
    /// regression makes the SECOND call take the `Ok` arm — which for
    /// `bind_transparent` would additionally drag a real socket bind into the
    /// default lane.
    #[test]
    fn armed_fault_is_standing_and_fires_on_every_call() {
        let sut = SimMtlsIntercept::new();
        sut.script_bind_fault(SimInterceptFault::TransparentListener { errno: libc::EADDRINUSE });

        let first = drive_expecting_err(&sut, Method::BindTransparent);
        let second = drive_expecting_err(&sut, Method::BindTransparent);

        assert_err_shape(&first, ExpectedErr::TransparentListener(libc::EADDRINUSE));
        assert_err_shape(&second, ExpectedErr::TransparentListener(libc::EADDRINUSE));
    }

    /// S-MIF-08 — `clear_faults` disarms ALL three slots, and clearing an
    /// already-disarmed double is a benign no-op.
    ///
    /// Universe: the `Result`s from `install_outbound` / `install_inbound`
    /// after the first `clear_faults()` (both `Ok`, each handing back a guard),
    /// and the same two after a SECOND `clear_faults()` (still `Ok`).
    ///
    /// `bind_transparent` is deliberately NOT re-driven after the clear — its
    /// `Ok` arm binds a real plain loopback socket (DFS-5) and would push this
    /// scenario into the integration lane. That its slot is cleared is covered
    /// indirectly by S-MIF-13 and directly at integration lane by S-MIF-09.
    ///
    /// The second `clear_faults()` is the fault state machine's
    /// illegal-event-from-the-disarmed-state case (C2b), asserted as a benign
    /// no-op rather than a panic or a state flip.
    #[test]
    fn clear_faults_disarms_every_slot_and_is_idempotent() {
        let sut = SimMtlsIntercept::new();
        sut.script_bind_fault(SimInterceptFault::TransparentListener { errno: libc::EPERM });
        sut.script_outbound_fault(SimInterceptFault::TproxyInstall {
            reason: "nft: command not found".to_owned(),
        });
        sut.script_inbound_fault(SimInterceptFault::TproxyInstall {
            reason: "ip rule add exited 2".to_owned(),
        });

        sut.clear_faults();
        sut.install_outbound("veth-alloc0", 4001).expect("clear_faults disarms the outbound slot");
        sut.install_inbound(VIRT, 4002).expect("clear_faults disarms the inbound slot");

        sut.clear_faults();
        sut.install_outbound("veth-alloc0", 4001)
            .expect("a second clear_faults leaves the outbound slot disarmed");
        sut.install_inbound(VIRT, 4002)
            .expect("a second clear_faults leaves the inbound slot disarmed");
    }

    /// S-MIF-13 — the three fault slots are INDEPENDENT: arming exactly one
    /// leaves the others on their success arms, across the three I/O-free
    /// directions.
    ///
    /// | Armed slot | `install_outbound` | `install_inbound` |
    /// |---|---|---|
    /// | `bind_fault` | `Ok(guard)` | `Ok(guard)` |
    /// | `outbound_fault` | `Err` | `Ok(guard)` |
    /// | `inbound_fault` | `Ok(guard)` | `Err` |
    ///
    /// Universe: the two `Result`s from `install_outbound` / `install_inbound`
    /// per direction.
    ///
    /// This is the ONLY scenario separating the slots. An implementation
    /// sharing one slot across all three methods, or a copy-paste bug pointing
    /// two scripting helpers at the same field, would still pass
    /// S-MIF-06/07/08 — each of those arms exactly one slot and reads back the
    /// same method.
    ///
    /// Named coverage gap (deliberate): the FOURTH direction — arm an install
    /// fault, confirm `bind_transparent` still takes its `Ok` arm — requires a
    /// real socket bind (DFS-5) and is therefore not default-lane. Not
    /// authored here.
    #[test]
    fn arming_one_slot_leaves_the_others_on_their_success_arms() {
        // Direction 1 — arming the BIND slot leaks to neither install.
        let sut = SimMtlsIntercept::new();
        sut.script_bind_fault(SimInterceptFault::TransparentListener { errno: libc::EPERM });
        sut.install_outbound("veth-alloc0", 4001)
            .expect("a bind fault does not leak into install_outbound");
        sut.install_inbound(VIRT, 4002).expect("a bind fault does not leak into install_inbound");

        // Direction 2 — arming the OUTBOUND slot refuses only `install_outbound`.
        let sut = SimMtlsIntercept::new();
        sut.script_outbound_fault(SimInterceptFault::TproxyInstall {
            reason: "nft add rule exited 1".to_owned(),
        });
        let got = drive_expecting_err(&sut, Method::InstallOutbound);
        assert_err_shape(&got, ExpectedErr::TproxyInstall("nft add rule exited 1"));
        sut.install_inbound(VIRT, 4002)
            .expect("an outbound fault does not leak into install_inbound");

        // Direction 3 — arming the INBOUND slot refuses only `install_inbound`.
        let sut = SimMtlsIntercept::new();
        sut.script_inbound_fault(SimInterceptFault::TproxyInstall {
            reason: "ip rule add exited 2".to_owned(),
        });
        sut.install_outbound("veth-alloc0", 4001)
            .expect("an inbound fault does not leak into install_outbound");
        let got = drive_expecting_err(&sut, Method::InstallInbound);
        assert_err_shape(&got, ExpectedErr::TproxyInstall("ip rule add exited 2"));
    }
}
