//! Quint conformance driver QC-4 — `relay_exchange` (public-ingress-gateway,
//! ADR-0239, ADR-0245, ADR-0246, ADR-0247, ADR-0253, ADR-0254).
//!
//! Replays `specs/quint/public-ingress-gateway/relay_exchange.qnt`,
//! conformance main `relay_conf` (the configuration of `relay_exchange` with
//! `REC = true`, recording `conf_step`; the stutter `idle` exists only in the
//! exhaustive main `relay_ok`, so no step maps to "no call"), against the
//! production `ExchangeMachine`. The owner is pure: the driver imports no
//! `Sim*` item. The spec is referenced in place.
//!
//! The driver struct holds the production owner and rebuilds it with
//! `ExchangeMachine::new()` at `init`. Every action calls exactly one owner
//! method (feature-delta, "Pinned observable projection for Quint
//! conformance", the only home of both tables):
//!
//! | Action | `ExchangeMachine` call |
//! |---|---|
//! | `init` | `ExchangeMachine::new()` |
//! | `read_head(n)` | `begin_request(false, BodyFraming::Length(n))` |
//! | `req_unit` | `request_octets_moved(1)` |
//! | `resp_head(ld, n)` | `response_head(Length(n))` when `ld`, else `response_head(UntilClose)` |
//! | `resp_unit` | `response_octets_moved(1)` |
//! | `up_close` | `peer_closed(Side::Upstream, true)` |
//! | `conclude` | `finish()`; the `Disposition` is checked against `reusable()` evaluated before the call |
//! | `p_close` | `peer_closed(Side::Public, true)` |
//! | `drain_begin` | `drain_begin()` |
//! | `force_abort` | `force_abort()` |
//!
//! Projection (`State::from_driver`), seven model variables read through six
//! accessors, nothing else:
//!
//! | Model variable | Accessor | Mapping |
//! |---|---|---|
//! | `phase` | `phase()` | `Idle` / `Relay` / `Closed` one to one |
//! | `req_left` | `request_remaining()` | `Exactly(k)` is `k`; `None` is 0 |
//! | `resp_state` | `response_state()` | `Pending` / `Body` / `Complete` as `RNone` / `RBody` / `RDone` |
//! | `resp_left` | `response_remaining()` | `Exactly(k)` is `k`; `None` and `UntilClose` are 0 |
//! | `len_delim` | `response_remaining()` | true unless the value is `UntilClose` |
//! | `draining` | `is_draining()` | boolean |
//! | `p_eof` | `public_half_closed()` | boolean |
//!
//! `exch`, `overread` and `ud_units` are model-only and not compared.
//! `reusable()` is the derived accessor of the `conclude` handler's
//! precondition, not a projected field.
//!
//! Step decoding: `Config::nondet = &["relay_conf::relay_exchange::conf_step"]`
//! (the qualified ITF name); the state projection matches
//! `relay_conf::relay_exchange::st`.
//!
//! Every named run of the module is pinned with
//! `#[quint_test(max_samples = 1)]` (a named run is deterministic: one sample
//! replays it); `#[quint_run]` replays randomized traces (seed printed on
//! failure). The drivers run against the production `ExchangeMachine`.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    reason = "a conformance driver asserts its own preconditions; protocol names are prose"
)]

use anyhow::{anyhow, bail, ensure};
use itf::Value;
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;

use super::exchange::{
    BodyFraming, Disposition, ExchangeMachine, ExchangePhase, Remaining, ResponseState, Side,
};

/// The conformance main this driver replays.
const MAIN: &str = "relay_conf";
/// The module that declares the compared variables.
const MODULE: &str = "relay_exchange";

/// The production owner under conformance.
struct RelayConfDriver {
    machine: ExchangeMachine,
}

impl Default for RelayConfDriver {
    fn default() -> Self {
        Self { machine: ExchangeMachine::new() }
    }
}

fn violation<T: std::fmt::Debug>(action: &str, error: T) -> anyhow::Error {
    anyhow!("the owner refused the model-legal action `{action}`: {error:?}")
}

fn unit_count(value: i64, what: &str) -> Result<u64> {
    u64::try_from(value).map_err(|_| anyhow!("the model's {what} {value} is negative"))
}

impl RelayConfDriver {
    const fn init(&mut self) {
        self.machine = ExchangeMachine::new();
    }

    fn read_head(&mut self, n: i64) -> Result {
        let n = unit_count(n, "request length")?;
        self.machine
            .begin_request(false, BodyFraming::Length(n))
            .map_err(|error| violation("read_head", error))
    }

    fn req_unit(&mut self) -> Result {
        self.machine.request_octets_moved(1).map_err(|error| violation("req_unit", error))
    }

    fn resp_head(&mut self, ld: bool, n: i64) -> Result {
        let framing = if ld {
            BodyFraming::Length(unit_count(n, "response length")?)
        } else {
            BodyFraming::UntilClose
        };
        self.machine.response_head(framing).map_err(|error| violation("resp_head", error))
    }

    fn resp_unit(&mut self) -> Result {
        self.machine.response_octets_moved(1).map_err(|error| violation("resp_unit", error))
    }

    fn up_close(&mut self) {
        self.machine.peer_closed(Side::Upstream, true);
    }

    fn conclude(&mut self) -> Result {
        let expected =
            if self.machine.reusable() { Disposition::Reuse } else { Disposition::Close };
        let disposition = self.machine.finish().map_err(|error| violation("conclude", error))?;
        ensure!(
            disposition == expected,
            "finish() returned {disposition:?} but reusable() decided {expected:?} before the call"
        );
        Ok(())
    }

    fn p_close(&mut self) {
        self.machine.peer_closed(Side::Public, true);
    }

    fn drain_begin(&mut self) {
        self.machine.drain_begin();
    }

    const fn force_abort(&mut self) {
        self.machine.force_abort();
    }
}

impl Driver for RelayConfDriver {
    type State = ExchangeState;

    fn config() -> Config {
        Config { state: &[], nondet: &["relay_conf::relay_exchange::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            init => self.init(),
            read_head(n: i64) => self.read_head(n)?,
            req_unit => self.req_unit()?,
            resp_head(ld: bool, n: i64) => self.resp_head(ld, n)?,
            resp_unit => self.resp_unit()?,
            up_close => self.up_close(),
            conclude => self.conclude()?,
            p_close => self.p_close(),
            drain_begin => self.drain_begin(),
            force_abort => self.force_abort(),
        })
    }
}

/// The compared `relay_exchange` variables.
#[derive(Debug, PartialEq, Eq)]
struct ExchangeState {
    phase: String,
    req_left: i64,
    resp_state: String,
    resp_left: i64,
    len_delim: bool,
    draining: bool,
    p_eof: bool,
}

fn model_units(remaining: Remaining, half: &str) -> Result<i64> {
    match remaining {
        Remaining::None => Ok(0),
        Remaining::Exactly(k) => {
            i64::try_from(k).map_err(|_| anyhow!("the {half} remainder {k} exceeds the model"))
        }
        Remaining::ChunkedOpen => {
            bail!("the {half} remainder is ChunkedOpen, which the model does not express")
        }
        Remaining::UntilClose => {
            bail!("the {half} remainder is UntilClose where the model has a length")
        }
    }
}

impl State<RelayConfDriver> for ExchangeState {
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &RelayConfDriver) -> Result<Self> {
        let phase = match driver.machine.phase() {
            ExchangePhase::Idle => "Idle",
            ExchangePhase::Relay => "Relay",
            ExchangePhase::Closed => "Closed",
        };
        let resp_state = match driver.machine.response_state() {
            ResponseState::Pending => "RNone",
            ResponseState::Body => "RBody",
            ResponseState::Complete => "RDone",
        };
        let resp_left = match driver.machine.response_remaining() {
            Remaining::UntilClose => 0,
            other => model_units(other, "response")?,
        };
        Ok(Self {
            phase: phase.to_owned(),
            req_left: model_units(driver.machine.request_remaining(), "request")?,
            resp_state: resp_state.to_owned(),
            resp_left,
            len_delim: !matches!(driver.machine.response_remaining(), Remaining::UntilClose),
            draining: driver.machine.is_draining(),
            p_eof: driver.machine.public_half_closed(),
        })
    }
}

impl ExchangeState {
    /// The spec side: the record `relay_conf::relay_exchange::st`.
    fn from_value(state: &Value) -> std::result::Result<Self, ItfError> {
        let Value::Record(record) = state else {
            return Err(ItfError::Shape {
                expected: "a record of variables",
                found: state.clone(),
            });
        };
        let qualified = format!("{MAIN}::{MODULE}::st");
        let st = record.get(&qualified).ok_or(ItfError::MissingVar(qualified))?;
        let Value::Record(st) = st else {
            return Err(ItfError::Shape { expected: "the state record `st`", found: st.clone() });
        };
        let field = |name: &str| -> std::result::Result<&Value, ItfError> {
            st.get(name).ok_or_else(|| ItfError::MissingField(name.to_owned()))
        };
        let text = |name: &str| -> std::result::Result<String, ItfError> {
            match field(name)? {
                Value::String(text) => Ok(text.clone()),
                other => Err(ItfError::Shape { expected: "a string", found: other.clone() }),
            }
        };
        let int = |name: &str| -> std::result::Result<i64, ItfError> {
            let value = field(name)?;
            match value {
                Value::Number(n) => Ok(*n),
                Value::BigInt(big) => i64::try_from(big.get().clone()).map_err(|_| {
                    ItfError::Shape { expected: "a 64-bit int", found: value.clone() }
                }),
                other => Err(ItfError::Shape { expected: "an int", found: other.clone() }),
            }
        };
        let flag = |name: &str| -> std::result::Result<bool, ItfError> {
            match field(name)? {
                Value::Bool(flag) => Ok(*flag),
                other => Err(ItfError::Shape { expected: "a bool", found: other.clone() }),
            }
        };
        Ok(Self {
            phase: text("phase")?,
            req_left: int("req_left")?,
            resp_state: text("resp_state")?,
            resp_left: int("resp_left")?,
            len_delim: flag("len_delim")?,
            draining: flag("draining")?,
            p_eof: flag("p_eof")?,
        })
    }
}

impl<'de> Deserialize<'de> for ExchangeState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

/// Why a trace value does not have the shape the driver compares.
#[derive(Debug, thiserror::Error)]
enum ItfError {
    #[error("the trace has no `{0}` variable")]
    MissingVar(String),
    #[error("the trace's record has no `{0}` field")]
    MissingField(String),
    #[error("expected {expected} in the trace, found {found:?}")]
    Shape { expected: &'static str, found: Value },
}

// ---------------------------------------------------------------------
// Named runs — one per critical scenario of the `Formal protocol model`
// section, then randomized traces.
// ---------------------------------------------------------------------

/// Two length-delimited exchanges reuse the public connection; in the first
/// the response starts before the request body is finished.
/// CONTRACT_SHAPE: pure-function.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/relay_exchange.qnt",
    main = "relay_conf",
    test = "exchange_cl_keepalive",
    max_samples = 1
)]
fn relay_exchange_exchange_cl_keepalive() -> impl Driver {
    RelayConfDriver::default()
}

/// The response completes before the request body: close, no reuse.
/// CONTRACT_SHAPE: pure-function.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/relay_exchange.qnt",
    main = "relay_conf",
    test = "early_response_closes",
    max_samples = 1
)]
fn relay_exchange_early_response_closes() -> impl Driver {
    RelayConfDriver::default()
}

/// A drain begun during the response body: the exchange completes, is not
/// reused, then closes.
/// CONTRACT_SHAPE: pure-function.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/relay_exchange.qnt",
    main = "relay_conf",
    test = "drain_with_inflight_body",
    max_samples = 1
)]
fn relay_exchange_drain_with_inflight_body() -> impl Driver {
    RelayConfDriver::default()
}

/// close_notify on the public leg after the request was fully read: the
/// response is still delivered, then the connection closes.
/// CONTRACT_SHAPE: pure-function.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/relay_exchange.qnt",
    main = "relay_conf",
    test = "half_close_after_request_read",
    max_samples = 1
)]
fn relay_exchange_half_close_after_request_read() -> impl Driver {
    RelayConfDriver::default()
}

/// close_notify from the upstream mid length-delimited response body: both
/// legs abort.
/// CONTRACT_SHAPE: pure-function.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/relay_exchange.qnt",
    main = "relay_conf",
    test = "upstream_close_mid_body_aborts",
    max_samples = 1
)]
fn relay_exchange_upstream_close_mid_body_aborts() -> impl Driver {
    RelayConfDriver::default()
}

/// A close-delimited response: units relayed, the upstream's clean close ends
/// it, the connection is not reused.
/// CONTRACT_SHAPE: pure-function.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/relay_exchange.qnt",
    main = "relay_conf",
    test = "close_delimited_clean_end",
    max_samples = 1
)]
fn relay_exchange_close_delimited_clean_end() -> impl Driver {
    RelayConfDriver::default()
}

/// close_notify on the public leg while the request body is unfinished:
/// abort.
/// CONTRACT_SHAPE: pure-function.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/relay_exchange.qnt",
    main = "relay_conf",
    test = "public_close_mid_request_aborts",
    max_samples = 1
)]
fn relay_exchange_public_close_mid_request_aborts() -> impl Driver {
    RelayConfDriver::default()
}

/// Randomized traces of the conformance main (seed printed on failure).
/// CONTRACT_SHAPE: pure-function.
#[quint_run(
    spec = "../../specs/quint/public-ingress-gateway/relay_exchange.qnt",
    main = "relay_conf"
)]
fn relay_exchange_relay_conf_randomized_traces() -> impl Driver {
    RelayConfDriver::default()
}
