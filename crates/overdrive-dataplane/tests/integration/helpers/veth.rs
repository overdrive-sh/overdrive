//! RAII veth-pair fixture for Tier 3 integration tests.
//!
//! Creates a `veth0` ↔ `veth1` peer pair via `ip(8)`, brings both ends
//! up, and tears the pair down on `Drop` regardless of test outcome.
//! Idempotent on `create()` — best-effort cleanup of leftover state from
//! a prior aborted run before issuing the `add` command.
//!
//! Capability gating: `ip link add … type veth` requires
//! `CAP_NET_ADMIN`. Unprivileged callers receive
//! [`VethError::CapNetAdminRequired`]; the test caller is expected to
//! return early with a skip message rather than panic, matching the
//! capability-gating convention in
//! `crates/overdrive-worker/tests/integration/exec_driver/`.

#![cfg(target_os = "linux")]
#![allow(clippy::expect_used)]

use std::process::{Command, Output};

/// Errors from veth-pair lifecycle. Distinct variants per
/// `.claude/rules/development.md` § Errors so the test caller can branch
/// on capability vs setup failure.
#[derive(Debug)]
pub enum VethError {
    /// `ip(8)` rejected the operation with EPERM/EACCES — the running
    /// process lacks `CAP_NET_ADMIN`. Tests skip rather than fail.
    CapNetAdminRequired,
    /// `ip(8)` failed for any other reason (binary missing, kernel
    /// rejection, peer-name conflict, …). Carries stderr for diagnosis.
    IpCommand { args: String, stderr: String, status: Option<i32> },
    /// Spawning `ip(8)` itself failed — typically the binary is not on
    /// `$PATH`. Distinct from `IpCommand` so the diagnostic can name the
    /// underlying I/O cause directly.
    Spawn(std::io::Error),
}

impl std::fmt::Display for VethError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CapNetAdminRequired => {
                f.write_str("ip(8) returned EPERM/EACCES — CAP_NET_ADMIN required")
            }
            Self::IpCommand { args, stderr, status } => {
                write!(f, "ip {args} failed (status={status:?}): {}", stderr.trim())
            }
            Self::Spawn(e) => write!(f, "ip(8) spawn failed: {e}"),
        }
    }
}

impl std::error::Error for VethError {}

/// RAII veth pair. `Drop` issues a best-effort `ip link del` on the
/// host-side end; the kernel automatically removes the peer when the
/// pair end is destroyed.
pub struct VethPair {
    /// Host-side endpoint — XDP attaches here.
    pub host: String,
    /// Peer endpoint — frames are injected here.
    pub peer: String,
}

impl VethPair {
    /// Create a fresh veth pair with the supplied names. If either
    /// endpoint already exists from a prior aborted run, tear it down
    /// first (best-effort) before issuing the add command. Brings both
    /// ends `up` so XDP attach and packet sendto both succeed.
    pub fn create(host: &str, peer: &str) -> Result<Self, VethError> {
        // Best-effort cleanup of leftover state. `ip link del` on a
        // missing iface returns non-zero — ignored. We only care about
        // the post-condition: neither name resolves.
        let _ = Command::new("ip").args(["link", "del", host]).output();
        let _ = Command::new("ip").args(["link", "del", peer]).output();

        run_ip(["link", "add", host, "type", "veth", "peer", "name", peer])?;
        run_ip(["link", "set", host, "up"])?;
        run_ip(["link", "set", peer, "up"])?;

        Ok(Self { host: host.to_owned(), peer: peer.to_owned() })
    }
}

impl Drop for VethPair {
    fn drop(&mut self) {
        // Best-effort teardown — ignore exit status. The kernel removes
        // the peer when the host side is destroyed.
        let _ = Command::new("ip").args(["link", "del", &self.host]).output();
    }
}

/// Spawn `ip <args>`, classify the result. EPERM/EACCES on stderr (or
/// the `Operation not permitted` text the iproute2 wrapper prints) maps
/// to [`VethError::CapNetAdminRequired`]; other non-zero exits map to
/// [`VethError::IpCommand`].
fn run_ip<I, S>(args: I) -> Result<Output, VethError>
where
    I: IntoIterator<Item = S> + Clone,
    S: AsRef<std::ffi::OsStr>,
{
    let arg_str = args
        .clone()
        .into_iter()
        .map(|s| s.as_ref().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    let out = Command::new("ip").args(args).output().map_err(VethError::Spawn)?;
    if out.status.success() {
        return Ok(out);
    }
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    // iproute2 surfaces EPERM as "Operation not permitted" and EACCES
    // as "Permission denied". Either signal a missing CAP_NET_ADMIN.
    if stderr.contains("Operation not permitted") || stderr.contains("Permission denied") {
        return Err(VethError::CapNetAdminRequired);
    }
    Err(VethError::IpCommand { args: arg_str, stderr, status: out.status.code() })
}
