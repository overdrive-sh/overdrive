//! Serve lifetime owner — the process-lifetime boundary of `overdrive serve`.
//!
//! [`crate::commands::serve::run`] starts the control plane and returns a
//! [`ServeHandle`]. This module owns what happens next: it waits for either
//! the control plane's typed internal shutdown request
//! ([`ServeHandle::shutdown_requested`]) or an operator stop signal, drives the
//! matching shutdown, and returns a typed [`ServeExit`] whose
//! [`ServeExit::exit_code`] the binary maps to the process exit status.
//!
//! Both nondeterministic inputs are injected at construction:
//!
//! - the operator stop-signal source ([`ServeSignals`]) — production binds
//!   real `SIGINT` / `SIGTERM` through [`OsServeSignals`]; tests drive their
//!   own source;
//! - the [`Clock`] that measures the fail-stop outer bound — production binds
//!   `SystemClock`; tests bind a simulated clock.
//!
//! `main.rs` constructs [`ServeLifetime`] and calls [`ServeLifetime::run`]; it
//! carries no selection, bound, or exit-status logic of its own, so the
//! library path the tests drive is the production path.
//!
//! Contract (`docs/feature/netns-density-295/feature-delta.md`, internal
//! fail-stop request to the CLI; ADR-0124): the internal request is selected
//! before the operator signal when both are ready; an operator stop keeps the
//! normal graceful shutdown and exit status `0`; a shared guest-network
//! fail-stop runs graceful shutdown under a hard [`FAIL_STOP_OUTER_BOUND`] and
//! exits status `1` whether shutdown drains, returns a typed error, or is
//! abandoned at the bound.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::guest_network::ServeShutdownRequest;
use overdrive_core::traits::clock::Clock;

use crate::commands::serve::ServeHandle;
use crate::http_client::CliError;

/// Hard outer bound on shutdown after a shared guest-network fail-stop
/// request. Measured on the injected [`Clock`].
pub const FAIL_STOP_OUTER_BOUND: Duration = Duration::from_secs(10);

/// An operator stop signal observed by the serve lifetime owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServeSignal {
    /// `SIGINT`.
    Interrupt,
    /// `SIGTERM`.
    Terminate,
    /// Killed mode: the in-process stand-in for an uncatchable `SIGKILL`.
    /// [`OsServeSignals`] never produces it. On receipt the lifetime owner
    /// abandons the serve owner without graceful shutdown or workload
    /// cleanup (see [`ServeExit::Killed`]).
    #[cfg(feature = "integration-tests")]
    Kill,
}

impl ServeSignal {
    /// Canonical signal name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Interrupt => "SIGINT",
            Self::Terminate => "SIGTERM",
            #[cfg(feature = "integration-tests")]
            Self::Kill => "SIGKILL",
        }
    }
}

/// Source of operator stop signals for [`ServeLifetime`].
///
/// # Contract
///
/// - `recv` resolves with the next stop signal delivered to the process and
///   stays pending while none has been delivered. It never resolves spuriously
///   and never busy-loops once the underlying source is exhausted — an
///   exhausted source stays pending forever.
/// - `recv` is cancellation safe: dropping the future before it resolves loses
///   no signal; a signal is consumed only by the poll that returns it.
pub trait ServeSignals: Send {
    /// Wait for the next operator stop signal.
    fn recv(&mut self) -> impl Future<Output = ServeSignal> + Send;
}

/// Production [`ServeSignals`] binding over the process's real `SIGINT` and
/// `SIGTERM` dispositions.
#[derive(Debug)]
pub struct OsServeSignals {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}

impl OsServeSignals {
    /// Install `SIGINT` and `SIGTERM` handlers on the current Tokio runtime.
    ///
    /// # Errors
    ///
    /// Returns the OS error when a handler cannot be registered.
    pub fn install() -> std::io::Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};
        Ok(Self {
            interrupt: signal(SignalKind::interrupt())?,
            terminate: signal(SignalKind::terminate())?,
        })
    }
}

impl ServeSignals for OsServeSignals {
    async fn recv(&mut self) -> ServeSignal {
        tokio::select! {
            biased;
            Some(()) = self.interrupt.recv() => ServeSignal::Interrupt,
            Some(()) = self.terminate.recv() => ServeSignal::Terminate,
            else => std::future::pending().await,
        }
    }
}

/// How shutdown ended after a shared guest-network fail-stop request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailStopCleanup {
    /// Graceful shutdown completed (successfully or with a typed, logged
    /// error) before [`FAIL_STOP_OUTER_BOUND`] elapsed.
    DrainedBeforeExit,
    /// [`FAIL_STOP_OUTER_BOUND`] elapsed first; shutdown was abandoned and the
    /// binary exits without awaiting runtime/task teardown.
    AbandonedAtExit,
}

impl FailStopCleanup {
    /// Canonical `cleanup=` evidence value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DrainedBeforeExit => "drained_before_exit",
            Self::AbandonedAtExit => "abandoned_at_exit",
        }
    }
}

/// Typed outcome of one serve lifetime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServeExit {
    /// An operator stop signal was received and graceful shutdown completed.
    Stopped {
        /// The signal that ended the lifetime.
        signal: ServeSignal,
    },
    /// The control plane requested shutdown because shared guest-network
    /// ownership could not be recovered.
    SharedGuestNetworkFailStop {
        /// The exact typed request the control plane produced.
        request: ServeShutdownRequest,
        /// Whether shutdown drained or was abandoned at the outer bound.
        cleanup: FailStopCleanup,
    },
    /// Killed mode: the serve owner was abandoned in-process without graceful
    /// shutdown or workload cleanup, leaving workload processes and host
    /// kernel state for the next boot's reclamation path.
    #[cfg(feature = "integration-tests")]
    Killed,
}

impl ServeExit {
    /// The process exit status this outcome maps to.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        match self {
            Self::Stopped { .. } => 0,
            Self::SharedGuestNetworkFailStop { .. } => 1,
            #[cfg(feature = "integration-tests")]
            Self::Killed => 137,
        }
    }
}

enum LifetimeEvent {
    Internal(ServeShutdownRequest),
    Signal(ServeSignal),
}

/// The `overdrive serve` process-lifetime owner.
pub struct ServeLifetime<S> {
    signals: S,
    clock: Arc<dyn Clock>,
}

impl<S: ServeSignals> ServeLifetime<S> {
    /// Construct the lifetime owner over its injected signal source and clock.
    #[must_use]
    pub fn new(signals: S, clock: Arc<dyn Clock>) -> Self {
        Self { signals, clock }
    }

    /// Own `handle` until the control plane requests shutdown or an operator
    /// stop signal arrives, drive the matching shutdown, and return the typed
    /// outcome.
    ///
    /// When the internal request and an operator signal are both ready, the
    /// internal request wins.
    ///
    /// # Errors
    ///
    /// Returns the typed [`CliError::ServerShutdown`] when graceful shutdown
    /// after an operator stop signal fails. A shared guest-network fail-stop
    /// never returns `Err`: its shutdown result is logged and the outcome is
    /// always [`ServeExit::SharedGuestNetworkFailStop`].
    pub async fn run(mut self, mut handle: ServeHandle) -> Result<ServeExit, CliError> {
        let event = tokio::select! {
            biased;
            request = handle.shutdown_requested() => LifetimeEvent::Internal(request),
            signal = self.signals.recv() => LifetimeEvent::Signal(signal),
        };
        match event {
            LifetimeEvent::Internal(request) => {
                Ok(fail_stop(self.clock.as_ref(), handle, request).await)
            }
            LifetimeEvent::Signal(signal) => stop(handle, signal).await,
        }
    }
}

async fn fail_stop(
    clock: &dyn Clock,
    handle: ServeHandle,
    request: ServeShutdownRequest,
) -> ServeExit {
    tracing::error!(
        name: "serve.shared_guest_network_fail_stop",
        request = ?request,
        outer_bound = ?FAIL_STOP_OUTER_BOUND,
        "shared guest-network fail-stop requested; shutting down within the outer bound"
    );
    let cleanup = tokio::select! {
        biased;
        result = handle.shutdown() => {
            if let Err(error) = result {
                tracing::error!(
                    name: "serve.fail_stop_shutdown_failed",
                    error = %error,
                    "graceful shutdown after shared guest-network fail-stop returned a typed error"
                );
            }
            FailStopCleanup::DrainedBeforeExit
        }
        () = clock.sleep(FAIL_STOP_OUTER_BOUND) => FailStopCleanup::AbandonedAtExit,
    };
    tracing::error!(
        name: "guest_network.shared_owner_fail_stop",
        cleanup = cleanup.as_str(),
        request = ?request,
        "shared guest-network fail-stop shutdown ended"
    );
    ServeExit::SharedGuestNetworkFailStop { request, cleanup }
}

async fn stop(handle: ServeHandle, signal: ServeSignal) -> Result<ServeExit, CliError> {
    match signal {
        ServeSignal::Interrupt | ServeSignal::Terminate => {
            tracing::info!(signal = signal.as_str(), "{} received; shutting down", signal.as_str());
            handle.shutdown().await?;
            Ok(ServeExit::Stopped { signal })
        }
        #[cfg(feature = "integration-tests")]
        ServeSignal::Kill => {
            handle.kill_for_test().await;
            Ok(ServeExit::Killed)
        }
    }
}
