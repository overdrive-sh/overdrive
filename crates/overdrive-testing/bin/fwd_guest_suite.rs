//! Host-side dispatcher for the `overdrive-fwd` module suite and the per-test
//! guest attempts the Metal test wrapper starts. The implementation lives in
//! [`overdrive_testing::guest_suite`] so the dispatcher and the
//! `#[overdrive_test(guest = "…")]` attribute share one guest boot, copy-in and
//! exec path. Test-support executable only; never shipped by a production crate.

#![cfg(target_os = "linux")]

/// Every guest-placed test whose module path carries a `quint_*` segment runs
/// the Quint CLI, so it is staged the prepared runtime; the model it resolves
/// relative to its working directory is declared once per (binary, module
/// prefix) here. A guest-placed `quint_*` module absent from this table fails
/// closed at staging instead of surfacing later as a failed Quint command
/// inside the guest.
const QUINT_GUEST_MODULES: &[(&str, &str, &str)] = &[(
    "overdrive-gateway",
    "application::quint_gateway_application::",
    "../../specs/quint/public-ingress-gateway/gateway_application.qnt",
)];

fn main() {
    overdrive_testing::guest_suite::run_dispatcher(QUINT_GUEST_MODULES);
}
