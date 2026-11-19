//! The retained record: schema version 1, one JSON object per line (ADR-0301, DDD-6).
//!
//! The typed schema is the one source of the writer's output and of the validator's parse:
//! unknown fields are refused, no required field is ever omitted to mean `null`, and the
//! closed vocabularies are enums. [`validate`] adds the semantic rules in line order.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::histogram::{Histogram, NotStorable, RAW_VALUE_LIMIT, storable};
use crate::registry::{GuestClock, ServiceClock};
use crate::{Error, Result};

pub(crate) const SCHEMA_VERSION: u32 = 1;
pub(crate) const METHODOLOGY_VERSION: u32 = 1;

pub(crate) const HOST_KEYS: [&str; 9] = [
    "kernel",
    "cpu_model",
    "cpu_count",
    "governor",
    "smt",
    "boost",
    "bpf_stats_enabled",
    "clocksource",
    "irq_time_accounting",
];

/// Check a whole record: the typed schema of every line, then the semantic rules and the
/// line order. A violation names its 1-based line and the offending field or name.
pub fn validate(text: &str) -> Result<()> {
    parse(text).map(drop)
}

/// The lines of a record that passed [`validate`], in order.
pub(crate) fn parse(text: &str) -> Result<Vec<Line>> {
    let mut checker = Checker::Fresh;
    let mut lines = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let line: Line =
            serde_json::from_str(raw).map_err(|source| invalid(number, parse_reason(&source)))?;
        checker.accept(&line).map_err(|reason| invalid(number, reason))?;
        lines.push(line);
    }
    if lines.is_empty() {
        return Err(invalid(1, "the record is empty; it must begin with run_started".to_owned()));
    }
    Ok(lines)
}

const fn invalid(line: usize, reason: String) -> Error {
    Error::InvalidRecord { line, reason }
}

/// `serde_json` appends ` at line L column C`, which names a position in the single parsed
/// line; the record's own line number is the one that matters.
fn parse_reason(error: &serde_json::Error) -> String {
    let message = error.to_string();
    match message.rsplit_once(" at line ") {
        Some((reason, _)) => reason.to_owned(),
        None => message,
    }
}

fn nullable<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

// ---------------------------------------------------------------------------------------
// The typed schema.

/// One line of the record; field `t` is the line type.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(crate) enum Line {
    RunStarted(Box<RunStarted>),
    Canary(Canary),
    Replicate(Box<ReplicateLine>),
    RunFinished(RunFinished),
    RunRefused(RunRefused),
    GuestObserved(GuestObserved),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceClass {
    Maintained,
    Spike,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Substrate {
    Native,
    Guest,
}

impl Substrate {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Guest => "guest",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BpfStatsMode {
    PerProcess,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ClockSource {
    #[serde(rename = "CLOCK_MONOTONIC_RAW")]
    MonotonicRaw,
}

impl ClockSource {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::MonotonicRaw => "CLOCK_MONOTONIC_RAW",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Clock {
    pub(crate) host: ClockSource,
    #[serde(deserialize_with = "service_clock", serialize_with = "serialize_service_clock")]
    pub(crate) service: Option<ServiceClock>,
    #[serde(deserialize_with = "guest_clock", serialize_with = "serialize_guest_clock")]
    pub(crate) guest: Option<GuestClock>,
}

/// Read a clock source that is `null` or the one string of a closed vocabulary; any other
/// string is refused naming the `field`.
fn closed_clock<'de, D, T>(
    deserializer: D,
    field: &str,
    vocabulary: &[(&'static str, T)],
) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Copy,
{
    let Some(text) = nullable::<D, String>(deserializer)? else {
        return Ok(None);
    };
    if let Some((_, source)) = vocabulary.iter().find(|(pinned, _)| *pinned == text) {
        return Ok(Some(*source));
    }
    let pinned: Vec<String> = vocabulary.iter().map(|(pinned, _)| format!("`{pinned}`")).collect();
    Err(D::Error::custom(format!("{field} `{text}` is not null or {}", pinned.join(" or "))))
}

fn service_clock<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<ServiceClock>, D::Error> {
    let source = ServiceClock::InjectedMonotonic;
    closed_clock(deserializer, "clock.service", &[(source.as_str(), source)])
}

fn guest_clock<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<GuestClock>, D::Error> {
    let vocabulary =
        [GuestClock::InitElapsedMs, GuestClock::Monotonic].map(|source| (source.as_str(), source));
    closed_clock(deserializer, "clock.guest", &vocabulary)
}

#[allow(
    clippy::ref_option,
    clippy::trivially_copy_pass_by_ref,
    reason = "the signature serde's `serialize_with` calls"
)]
fn serialize_service_clock<S: serde::Serializer>(
    source: &Option<ServiceClock>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_some(&source.map(ServiceClock::as_str))
}

#[allow(
    clippy::ref_option,
    clippy::trivially_copy_pass_by_ref,
    reason = "the signature serde's `serialize_with` calls"
)]
fn serialize_guest_clock<S: serde::Serializer>(
    source: &Option<GuestClock>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_some(&source.map(GuestClock::as_str))
}

/// One value of the `host` block: a string, an absent source, or an unreadable one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum HostValue {
    Text(String),
    Absent { absent: String },
    Unreadable { unreadable: String },
}

impl HostValue {
    fn from_json(value: &serde_json::Value) -> std::result::Result<Self, &'static str> {
        const FORMS: &str =
            "must be a string, {\"absent\": <string>} or {\"unreadable\": <string>}";
        match value {
            serde_json::Value::String(text) => Ok(Self::Text(text.clone())),
            serde_json::Value::Object(map) if map.len() == 1 => {
                match (map.get("absent"), map.get("unreadable")) {
                    (Some(serde_json::Value::String(reason)), None) => {
                        Ok(Self::Absent { absent: reason.clone() })
                    }
                    (None, Some(serde_json::Value::String(error))) => {
                        Ok(Self::Unreadable { unreadable: error.clone() })
                    }
                    _ => Err(FORMS),
                }
            }
            _ => Err(FORMS),
        }
    }
}

// The guest benchmark vocabulary (DDD-4, DDD-7).

/// The guest's view of a run: what a replicate reports through
/// [`Replicate::report_guest_view`](crate::Replicate::report_guest_view).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestView {
    pub vcpus: std::num::NonZeroU8,
    pub memory_mib: GuestMemoryMib,
    pub kernel: HostValue,
    pub clocksource: HostValue,
    pub artifacts: GuestArtifacts,
}

/// The three staged files the kit copies into the guest image, each its SHA-256.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestArtifacts {
    pub overdrive_init: HostValue,
    pub overdrive_guest_peer: HostValue,
    pub ovd_fwd: HostValue,
}

/// A guest memory size in MiB: positive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GuestMemoryMib(std::num::NonZeroU32);

impl GuestMemoryMib {
    #[must_use]
    pub const fn new(mib: std::num::NonZeroU32) -> Self {
        Self(mib)
    }
}

/// Why a string is not a guest memory size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestMemoryMibError;

impl std::fmt::Display for GuestMemoryMibError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("not a guest memory size")
    }
}

impl std::error::Error for GuestMemoryMibError {}

impl std::fmt::Display for GuestMemoryMib {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::str::FromStr for GuestMemoryMib {
    type Err = GuestMemoryMibError;

    fn from_str(text: &str) -> std::result::Result<Self, Self::Err> {
        // Decimal digits only: `u32::from_str` would also accept a leading `+`.
        if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(GuestMemoryMibError);
        }
        let mib: u32 = text.parse().map_err(|_| GuestMemoryMibError)?;
        std::num::NonZeroU32::new(mib).map(Self).ok_or(GuestMemoryMibError)
    }
}

impl Serialize for GuestMemoryMib {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_u32(self.0.get())
    }
}

impl<'de> Deserialize<'de> for GuestMemoryMib {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let mib = u32::deserialize(deserializer)?;
        std::num::NonZeroU32::new(mib)
            .map(Self)
            .ok_or_else(|| D::Error::custom("memory_mib must be a positive integer, not 0"))
    }
}

/// What a guest cannot see of the host's settings, as the record states it.
pub(crate) const NOT_VISIBLE_FROM_A_GUEST: &str = "not visible from a guest";

pub(crate) const GUEST_SETTINGS: [&str; 3] = ["governor", "smt", "boost"];
const GUEST_ARTIFACTS: [&str; 3] = ["overdrive_init", "overdrive_guest_peer", "ovd_fwd"];

/// The optional `guest` block of `run_started` (DDD-2): what is true before any guest boots.
/// Every value is pinned (`launch` is `metal_host`, `boot` is `per_replicate`, `vcpu_scheduling`
/// is `unpinned`, and the three host settings are each absent: not visible from a guest), so the
/// block carries no data and the validator accepts exactly that one JSON object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GuestBlock;

impl GuestBlock {
    fn json() -> serde_json::Value {
        let absent = serde_json::json!({ "absent": NOT_VISIBLE_FROM_A_GUEST });
        serde_json::json!({
            "launch": "metal_host",
            "boot": "per_replicate",
            "vcpu_scheduling": "unpinned",
            "settings": { "governor": absent, "smt": absent, "boost": absent },
        })
    }

    fn check(raw: &BTreeMap<String, serde_json::Value>) -> std::result::Result<(), String> {
        const KEYS: [&str; 4] = ["launch", "boot", "vcpu_scheduling", "settings"];
        if let Some(key) = raw.keys().find(|key| !KEYS.contains(&key.as_str())) {
            return Err(format!("guest has unknown key `{key}`"));
        }
        for (key, pinned) in
            [("launch", "metal_host"), ("boot", "per_replicate"), ("vcpu_scheduling", "unpinned")]
        {
            match raw.get(key) {
                Some(serde_json::Value::String(text)) if text == pinned => {}
                Some(_) => return Err(format!("guest {key} must be `{pinned}`")),
                None => return Err(format!("guest is missing key `{key}`")),
            }
        }
        match raw.get("settings") {
            Some(serde_json::Value::Object(settings)) => Self::check_settings(settings),
            Some(_) => Err("guest settings must be an object".to_owned()),
            None => Err("guest is missing key `settings`".to_owned()),
        }
    }

    fn check_settings(
        settings: &serde_json::Map<String, serde_json::Value>,
    ) -> std::result::Result<(), String> {
        if let Some(key) = settings.keys().find(|key| !GUEST_SETTINGS.contains(&key.as_str())) {
            return Err(format!("guest settings has unknown key `{key}`"));
        }
        let pinned = serde_json::json!({ "absent": NOT_VISIBLE_FROM_A_GUEST });
        for key in GUEST_SETTINGS {
            match settings.get(key) {
                Some(value) if *value == pinned => {}
                Some(_) => {
                    return Err(format!(
                        "guest setting `{key}` must be exactly {{\"absent\": \"{NOT_VISIBLE_FROM_A_GUEST}\"}}"
                    ));
                }
                None => return Err(format!("guest settings is missing key `{key}`")),
            }
        }
        Ok(())
    }
}

impl Serialize for GuestBlock {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        Self::json().serialize(serializer)
    }
}

/// Read the `guest` key when it is present: the key is absent on a native record, so a present
/// `null` is refused naming `guest` (DDD-2).
fn present_guest<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<GuestBlock>, D::Error> {
    let raw = serde_json::Value::deserialize(deserializer)?;
    if raw.is_null() {
        return Err(D::Error::custom(
            "guest must be omitted, not null, unless the substrate is guest",
        ));
    }
    GuestBlock::deserialize(raw).map(Some).map_err(D::Error::custom)
}

impl<'de> Deserialize<'de> for GuestBlock {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
        Self::check(&raw).map_err(D::Error::custom)?;
        Ok(Self)
    }
}

/// The three staged artifacts of a `guest_observed` line, each a SHA-256 in one of the host
/// block's three forms.
#[derive(Debug, Serialize)]
pub(crate) struct ObservedArtifacts {
    pub(crate) overdrive_init: HostValue,
    pub(crate) overdrive_guest_peer: HostValue,
    pub(crate) ovd_fwd: HostValue,
}

/// The `guest_observed` line (DDD-2): the guest's view as the first reporting replicate
/// reported it.
#[derive(Debug, Serialize)]
pub(crate) struct GuestObserved {
    pub(crate) vcpus: std::num::NonZeroU8,
    pub(crate) memory_mib: GuestMemoryMib,
    pub(crate) kernel: HostValue,
    pub(crate) clocksource: HostValue,
    pub(crate) artifacts: ObservedArtifacts,
}

/// Whether `text` is a SHA-256 as the record writes it: 64 lowercase hex digits.
fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn observed_field<'a>(
    raw: &'a BTreeMap<String, serde_json::Value>,
    key: &str,
) -> std::result::Result<&'a serde_json::Value, String> {
    raw.get(key).ok_or_else(|| format!("guest_observed is missing key `{key}`"))
}

fn observed_value(
    raw: &BTreeMap<String, serde_json::Value>,
    key: &str,
) -> std::result::Result<HostValue, String> {
    HostValue::from_json(observed_field(raw, key)?)
        .map_err(|form| format!("guest_observed `{key}` {form}"))
}

fn observed_count<T>(
    raw: &BTreeMap<String, serde_json::Value>,
    key: &str,
    make: impl FnOnce(u64) -> Option<T>,
    range: &str,
) -> std::result::Result<T, String> {
    observed_field(raw, key)?
        .as_u64()
        .and_then(make)
        .ok_or_else(|| format!("guest_observed `{key}` must be an integer {range}"))
}

fn observed_artifacts(
    raw: &BTreeMap<String, serde_json::Value>,
) -> std::result::Result<ObservedArtifacts, String> {
    let serde_json::Value::Object(artifacts) = observed_field(raw, "artifacts")? else {
        return Err("guest_observed `artifacts` must be an object".to_owned());
    };
    if let Some(key) = artifacts.keys().find(|key| !GUEST_ARTIFACTS.contains(&key.as_str())) {
        return Err(format!("guest_observed artifacts has unknown key `{key}`"));
    }
    let digest = |key: &str| {
        let value = artifacts
            .get(key)
            .ok_or_else(|| format!("guest_observed artifacts is missing key `{key}`"))
            .and_then(|value| {
                HostValue::from_json(value)
                    .map_err(|form| format!("guest_observed artifact `{key}` {form}"))
            })?;
        match &value {
            HostValue::Text(text) if !is_sha256_hex(text) => Err(format!(
                "guest_observed artifact `{key}` must be 64 lowercase hex digits, found `{text}`"
            )),
            _ => Ok(value),
        }
    };
    Ok(ObservedArtifacts {
        overdrive_init: digest("overdrive_init")?,
        overdrive_guest_peer: digest("overdrive_guest_peer")?,
        ovd_fwd: digest("ovd_fwd")?,
    })
}

impl GuestObserved {
    fn check(raw: &BTreeMap<String, serde_json::Value>) -> std::result::Result<Self, String> {
        const KEYS: [&str; 5] = ["vcpus", "memory_mib", "kernel", "clocksource", "artifacts"];
        if let Some(key) = raw.keys().find(|key| !KEYS.contains(&key.as_str())) {
            return Err(format!("guest_observed has unknown key `{key}`"));
        }
        Ok(Self {
            vcpus: observed_count(
                raw,
                "vcpus",
                |count| u8::try_from(count).ok().and_then(std::num::NonZeroU8::new),
                "from 1 to 255",
            )?,
            memory_mib: observed_count(
                raw,
                "memory_mib",
                |count| u32::try_from(count).ok().and_then(std::num::NonZeroU32::new),
                "from 1 to 4294967295",
            )
            .map(GuestMemoryMib::new)?,
            kernel: observed_value(raw, "kernel")?,
            clocksource: observed_value(raw, "clocksource")?,
            artifacts: observed_artifacts(raw)?,
        })
    }
}

impl<'de> Deserialize<'de> for GuestObserved {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
        Self::check(&raw).map_err(D::Error::custom)
    }
}

/// The `host` block: exactly the nine pinned keys, each in one of the three forms.
#[derive(Debug, Serialize)]
pub(crate) struct Host(pub(crate) BTreeMap<String, HostValue>);

impl<'de> Deserialize<'de> for Host {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
        if let Some(key) = raw.keys().find(|key| !HOST_KEYS.contains(&key.as_str())) {
            return Err(D::Error::custom(format!("host has unknown key `{key}`")));
        }
        let mut entries = BTreeMap::new();
        for key in HOST_KEYS {
            let value = raw
                .get(key)
                .ok_or_else(|| D::Error::custom(format!("host is missing key `{key}`")))?;
            let parsed = HostValue::from_json(value)
                .map_err(|form| D::Error::custom(format!("host key `{key}` {form}")))?;
            entries.insert(key.to_owned(), parsed);
        }
        Ok(Self(entries))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Unit {
    Ns,
    Bytes,
    BytesPerSecond,
    Qps,
    Ratio,
    Count,
}

impl Unit {
    /// The unit a declaration names (`ns bytes bytes_per_second qps ratio count`).
    pub(crate) fn parse(name: &str) -> Option<Self> {
        [Self::Ns, Self::Bytes, Self::BytesPerSecond, Self::Qps, Self::Ratio, Self::Count]
            .into_iter()
            .find(|unit| unit.as_str() == name)
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Ns => "ns",
            Self::Bytes => "bytes",
            Self::BytesPerSecond => "bytes_per_second",
            Self::Qps => "qps",
            Self::Ratio => "ratio",
            Self::Count => "count",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileDecl {
    pub(crate) name: String,
    pub(crate) preparation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetricDecl {
    pub(crate) name: String,
    pub(crate) unit: Unit,
    pub(crate) table: String,
    pub(crate) series: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReferenceDecl {
    pub(crate) metric: String,
    pub(crate) value: f64,
    pub(crate) label: String,
    pub(crate) source: String,
}

/// The echo of the benchmark attribute.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Declaration {
    pub(crate) arms: Vec<String>,
    pub(crate) replicates: u32,
    pub(crate) profiles: Vec<ProfileDecl>,
    pub(crate) metrics: Vec<MetricDecl>,
    pub(crate) references: Vec<ReferenceDecl>,
}

impl Declaration {
    fn reference_arm(&self) -> Option<&str> {
        self.arms.first().map(String::as_str)
    }

    fn declares_profile(&self, name: &str) -> bool {
        self.profiles.iter().any(|profile| profile.name == name)
    }

    fn metric(&self, name: &str) -> Option<&MetricDecl> {
        self.metrics.iter().find(|metric| metric.name == name)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunStarted {
    pub(crate) schema: u32,
    pub(crate) methodology: u32,
    pub(crate) run_id: String,
    pub(crate) family: String,
    pub(crate) benchmark: String,
    pub(crate) evidence: EvidenceClass,
    pub(crate) substrate: Substrate,
    pub(crate) commit: String,
    pub(crate) dirty: bool,
    pub(crate) debug_assertions: bool,
    pub(crate) seed: u64,
    pub(crate) started: String,
    pub(crate) host: Host,
    pub(crate) clock: Clock,
    /// Present exactly when the substrate is `guest` (DDD-2).
    #[serde(default, deserialize_with = "present_guest", skip_serializing_if = "Option::is_none")]
    pub(crate) guest: Option<GuestBlock>,
    pub(crate) bpf_stats: BpfStatsMode,
    pub(crate) declaration: Declaration,
}

impl RunStarted {
    fn check(&self) -> std::result::Result<(), String> {
        if self.schema != SCHEMA_VERSION {
            return Err(format!("schema must be {SCHEMA_VERSION}, found {}", self.schema));
        }
        if self.methodology != METHODOLOGY_VERSION {
            return Err(format!(
                "methodology must be {METHODOLOGY_VERSION}, found {}",
                self.methodology
            ));
        }
        if self.declaration.arms.is_empty() {
            return Err("declaration.arms must name at least the reference arm".to_owned());
        }
        match (self.substrate, &self.guest) {
            (Substrate::Guest, None) => {
                Err("guest is required when the substrate is guest".to_owned())
            }
            (Substrate::Native, Some(_)) => {
                Err("guest is allowed only when the substrate is guest".to_owned())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CanaryPhase {
    Start,
    End,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Canary {
    pub(crate) phase: CanaryPhase,
    pub(crate) samples_ns: Vec<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Role {
    Arm,
    Control,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Ok,
    Failed,
}

/// One metric of one replicate: one of raw values, a histogram, failures or an absence; a series
/// metric may hold measured values (or a histogram) together with failures.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetricEntry {
    pub(crate) values: Vec<f64>,
    #[serde(deserialize_with = "nullable")]
    pub(crate) histogram: Option<Histogram>,
    pub(crate) failures: Vec<String>,
    #[serde(deserialize_with = "nullable")]
    pub(crate) absent: Option<String>,
}

impl MetricEntry {
    /// A series metric's stored form: raw values up to 1,000, a histogram of all of them above.
    /// Every value is kept; the switch is a representation choice, not a cap (DDD-5).
    pub(crate) fn from_series(values: &[f64]) -> std::result::Result<Self, NotStorable> {
        if values.len() > RAW_VALUE_LIMIT {
            return Ok(Self {
                values: Vec::new(),
                histogram: Some(Histogram::from_values(values)?),
                failures: Vec::new(),
                absent: None,
            });
        }
        if values.is_empty() {
            return Err(NotStorable::Empty);
        }
        let raw =
            values.iter().map(|value| storable(*value)).collect::<std::result::Result<_, _>>()?;
        Ok(Self { values: raw, histogram: None, failures: Vec::new(), absent: None })
    }

    /// A scalar metric's one value.
    pub(crate) fn scalar(value: f64) -> Self {
        Self { values: vec![value], histogram: None, failures: Vec::new(), absent: None }
    }

    /// A metric that could not be measured, once for each of `reasons`.
    pub(crate) const fn failures(reasons: Vec<String>) -> Self {
        Self { values: Vec::new(), histogram: None, failures: reasons, absent: None }
    }

    /// A metric that failed with the single `reason`.
    pub(crate) fn failed(reason: &str) -> Self {
        Self::failures(vec![reason.to_owned()])
    }

    /// A metric that does not apply to the replicate.
    pub(crate) fn absent(reason: &str) -> Self {
        Self {
            values: Vec::new(),
            histogram: None,
            failures: Vec::new(),
            absent: Some(reason.to_owned()),
        }
    }

    fn shape_error(&self, series: bool, exactly_one: bool) -> Option<String> {
        if let Some(histogram) = &self.histogram {
            if !series {
                return Some("a scalar metric never holds a histogram".to_owned());
            }
            if !self.values.is_empty() {
                return Some("a histogram's values must be empty".to_owned());
            }
            if let Some(error) = histogram.shape_error() {
                return Some(error);
            }
        }
        if self.values.len() > RAW_VALUE_LIMIT {
            return Some(format!("values holds more than {RAW_VALUE_LIMIT} values"));
        }
        if !series && self.values.len() > 1 {
            return Some("a scalar metric holds at most one value".to_owned());
        }
        if !exactly_one {
            return None;
        }
        let measured = !self.values.is_empty() || self.histogram.is_some();
        let failed = !self.failures.is_empty();
        let absent = self.absent.is_some();
        if series {
            // Measured values, failures or both (a partly measured series), or an absence alone.
            return (absent == (measured || failed)).then(|| {
                "a series metric entry must hold measured values, failures, both, or absent alone"
                    .to_owned()
            });
        }
        let held = [measured, failed, absent].into_iter().filter(|held| *held).count();
        (held != 1).then(|| {
            "a scalar metric entry must hold exactly one of a value, failures or absent".to_owned()
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Command {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    #[serde(deserialize_with = "nullable")]
    pub(crate) status: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplicateLine {
    pub(crate) profile: String,
    pub(crate) arm: String,
    pub(crate) block: u32,
    pub(crate) position: u32,
    pub(crate) role: Role,
    pub(crate) outcome: Outcome,
    #[serde(deserialize_with = "nullable")]
    pub(crate) failure: Option<String>,
    pub(crate) metrics: BTreeMap<String, MetricEntry>,
    pub(crate) notes: BTreeMap<String, String>,
    pub(crate) commands: Vec<Command>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunFinished {
    pub(crate) finished: String,
    pub(crate) replicates_ok: u64,
    pub(crate) replicates_failed: u64,
}

/// The terminal line of a refused run: a difference (`found` and `pinned`) or a `reason`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunRefused {
    pub(crate) setting: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) found: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pinned: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
}

impl RunRefused {
    fn check(&self) -> std::result::Result<(), String> {
        let difference = self.found.is_some() && self.pinned.is_some() && self.reason.is_none();
        let reason_only = self.found.is_none() && self.pinned.is_none() && self.reason.is_some();
        if difference || reason_only {
            return Ok(());
        }
        Err("run_refused must carry both found and pinned, or a reason, and nothing else"
            .to_owned())
    }
}

// ---------------------------------------------------------------------------------------
// The writer.

/// Append-only writer of a record: one JSON object per line, flushed per line.
pub(crate) struct RecordWriter<W: Write> {
    out: W,
    path: PathBuf,
}

impl<W: Write> RecordWriter<W> {
    pub(crate) const fn new(out: W, path: PathBuf) -> Self {
        Self { out, path }
    }

    /// Write `line` and flush it, so a killed run leaves every completed line on disk.
    pub(crate) fn append(&mut self, line: &Line) -> Result<()> {
        let mut text = serde_json::to_string(line).map_err(|source| self.fail(source.into()))?;
        text.push('\n');
        self.out
            .write_all(text.as_bytes())
            .and_then(|()| self.out.flush())
            .map_err(|source| self.fail(source))
    }

    fn fail(&self, source: std::io::Error) -> Error {
        Error::Record { path: self.path.clone(), source }
    }
}

// ---------------------------------------------------------------------------------------
// The semantic rules, in line order.

/// Where the record stands: before `run_started`, inside the run, or past its terminal line.
enum Checker {
    Fresh,
    Open(Open),
    Closed,
}

enum Stage {
    Started,
    Measuring,
    Measured,
}

enum Step {
    Continue,
    Terminal,
}

struct Open {
    declaration: Declaration,
    /// Whether `run_started` carries a `guest` block.
    guest: bool,
    /// Whether the record's one `guest_observed` line has been seen.
    view_seen: bool,
    stage: Stage,
    ok: u64,
    failed: u64,
}

impl Checker {
    fn accept(&mut self, line: &Line) -> std::result::Result<(), String> {
        match self {
            Self::Closed => Err("no line may follow the terminal line".to_owned()),
            Self::Fresh => {
                let Line::RunStarted(started) = line else {
                    return Err("the record must begin with run_started".to_owned());
                };
                started.check()?;
                *self = Self::Open(Open {
                    declaration: started.declaration.clone(),
                    guest: started.guest.is_some(),
                    view_seen: false,
                    stage: Stage::Started,
                    ok: 0,
                    failed: 0,
                });
                Ok(())
            }
            Self::Open(open) => {
                if matches!(open.accept(line)?, Step::Terminal) {
                    *self = Self::Closed;
                }
                Ok(())
            }
        }
    }
}

impl Open {
    fn accept(&mut self, line: &Line) -> std::result::Result<Step, String> {
        match (&self.stage, line) {
            (_, Line::RunStarted(_)) => Err("run_started may only be the first line".to_owned()),
            (_, Line::GuestObserved(_)) if !self.guest => {
                Err("guest_observed requires a run_started with a guest block".to_owned())
            }
            (Stage::Started, Line::GuestObserved(_)) => {
                Err("guest_observed may only follow the start canary".to_owned())
            }
            (Stage::Measuring, Line::GuestObserved(_)) => {
                if self.view_seen {
                    return Err("guest_observed may appear at most once".to_owned());
                }
                self.view_seen = true;
                Ok(Step::Continue)
            }
            (Stage::Started, Line::Canary(canary)) if canary.phase == CanaryPhase::Start => {
                self.stage = Stage::Measuring;
                Ok(Step::Continue)
            }
            (Stage::Started, Line::RunRefused(refused)) => {
                refused.check()?;
                Ok(Step::Terminal)
            }
            (Stage::Started, _) => {
                Err("run_started must be followed by the start canary or run_refused".to_owned())
            }
            (Stage::Measuring, Line::Replicate(replicate)) => {
                self.replicate(replicate)?;
                Ok(Step::Continue)
            }
            (Stage::Measuring, Line::Canary(canary)) if canary.phase == CanaryPhase::End => {
                self.stage = Stage::Measured;
                Ok(Step::Continue)
            }
            (Stage::Measuring, _) => Err("expected a replicate or the end canary".to_owned()),
            (Stage::Measured, Line::RunFinished(finished)) => {
                self.finish(finished)?;
                Ok(Step::Terminal)
            }
            (Stage::Measured, _) => Err("expected run_finished after the end canary".to_owned()),
        }
    }

    fn replicate(&mut self, replicate: &ReplicateLine) -> std::result::Result<(), String> {
        let declaration = &self.declaration;
        if !declaration.declares_profile(&replicate.profile) {
            return Err(format!("replicate names undeclared profile `{}`", replicate.profile));
        }
        if !declaration.arms.contains(&replicate.arm) {
            return Err(format!("replicate names undeclared arm `{}`", replicate.arm));
        }
        let reference = declaration.reference_arm().unwrap_or_default();
        if replicate.role == Role::Control && replicate.arm != reference {
            return Err(format!(
                "a control must carry the reference arm `{reference}` in arm, found `{}`",
                replicate.arm
            ));
        }
        match (replicate.outcome, &replicate.failure) {
            (Outcome::Failed, None) => {
                return Err("a failed replicate must carry a failure reason".to_owned());
            }
            (Outcome::Ok, Some(_)) => {
                return Err("an ok replicate must not carry a failure reason".to_owned());
            }
            _ => {}
        }
        for (name, entry) in &replicate.metrics {
            let Some(metric) = declaration.metric(name) else {
                return Err(format!("replicate names undeclared metric `{name}`"));
            };
            if let Some(error) = entry.shape_error(metric.series, replicate.outcome == Outcome::Ok)
            {
                return Err(format!("metric `{name}`: {error}"));
            }
        }
        if self.guest && replicate.outcome == Outcome::Ok && !self.view_seen {
            return Err(
                "the first ok replicate of a guest record must follow its guest_observed line"
                    .to_owned(),
            );
        }
        match replicate.outcome {
            Outcome::Ok => self.ok += 1,
            Outcome::Failed => self.failed += 1,
        }
        Ok(())
    }

    fn finish(&self, finished: &RunFinished) -> std::result::Result<(), String> {
        if finished.replicates_ok != self.ok {
            return Err(format!(
                "replicates_ok is {} but the record holds {} ok replicates",
                finished.replicates_ok, self.ok
            ));
        }
        if finished.replicates_failed != self.failed {
            return Err(format!(
                "replicates_failed is {} but the record holds {} failed replicates",
                finished.replicates_failed, self.failed
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "tests assert their own preconditions and compare exact stored floats"
)]
mod tests {
    use proptest::prelude::*;
    use serde_json::{Value, json};

    use super::*;

    struct Refusing;
    impl Write for Refusing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disk full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// The document's stated version is the version the runner stamps into every record.
    #[test]
    fn the_methodology_document_states_the_version_the_runner_stamps() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../benchmarks/methodology.md");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
        let stated = text
            .lines()
            .find_map(|line| line.strip_prefix("Methodology version "))
            .and_then(|rest| {
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                digits.parse::<u32>().ok()
            })
            .expect("the document opens a line with `Methodology version <n>`");
        assert_eq!(stated, METHODOLOGY_VERSION);
    }

    fn host() -> Value {
        let keys: BTreeMap<&str, &str> = HOST_KEYS.iter().map(|key| (*key, "x")).collect();
        json!(keys)
    }

    fn started(series: bool) -> Value {
        json!({
            "t": "run_started", "schema": 1, "methodology": 1,
            "run_id": "0123456789abcdef01234567", "family": "f", "benchmark": "b",
            "evidence": "maintained", "substrate": "native",
            "commit": "e3b42c92e0123456789abcdef0123456789abcde", "dirty": false,
            "debug_assertions": false, "seed": 1, "started": "2026-11-17T10:00:00Z",
            "host": host(),
            "clock": { "host": "CLOCK_MONOTONIC_RAW", "service": null, "guest": null },
            "bpf_stats": "off",
            "declaration": {
                "arms": ["arm"], "replicates": 9,
                "profiles": [{ "name": "default", "preparation": "" }],
                "metrics": [{ "name": "m", "unit": "ns", "table": "t", "series": series }],
                "references": []
            }
        })
    }

    fn canary(phase: &str) -> Value {
        json!({ "t": "canary", "phase": phase, "samples_ns": [1.0] })
    }

    fn replicate_of(entry: &MetricEntry) -> Value {
        json!({
            "t": "replicate", "profile": "default", "arm": "arm", "block": 0, "position": 0,
            "role": "arm", "outcome": "ok", "failure": null,
            "metrics": { "m": entry }, "notes": {}, "commands": []
        })
    }

    fn finished() -> Value {
        json!({ "t": "run_finished", "finished": "2026-11-17T10:05:00Z",
                "replicates_ok": 1, "replicates_failed": 0 })
    }

    fn record(lines: &[Value]) -> String {
        lines.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n") + "\n"
    }

    fn series_record(entry: &MetricEntry) -> String {
        record(&[started(true), canary("start"), replicate_of(entry), canary("end"), finished()])
    }

    #[test]
    fn a_series_of_exactly_1000_values_is_stored_raw_and_1001_as_a_histogram() {
        let thousand: Vec<f64> = (1..=1000).map(f64::from).collect();
        let raw = MetricEntry::from_series(&thousand).expect("storable");
        assert_eq!(raw.values.len(), 1000);
        assert!(raw.histogram.is_none());
        let over: Vec<f64> = (1..=1001).map(f64::from).collect();
        let binned = MetricEntry::from_series(&over).expect("storable");
        assert!(binned.values.is_empty());
        assert_eq!(binned.histogram.expect("histogram").count, 1001);
    }

    #[test]
    fn a_series_with_a_refused_value_or_no_value_is_not_storable() {
        assert_eq!(MetricEntry::from_series(&[]), Err(NotStorable::Empty));
        assert!(matches!(MetricEntry::from_series(&[1.0, -2.0]), Err(NotStorable::Value(_))));
        let mut over: Vec<f64> = vec![1.0; 1001];
        over[500] = f64::NAN;
        assert!(matches!(MetricEntry::from_series(&over), Err(NotStorable::Value(_))));
    }

    #[test]
    fn the_writer_appends_one_flushed_line_and_names_the_path_when_it_fails() {
        let line: Line = serde_json::from_value(canary("start")).expect("line");
        let mut written = RecordWriter::new(Vec::new(), PathBuf::from("record.jsonl"));
        written.append(&line).expect("append");
        written.append(&line).expect("append");
        let text = String::from_utf8(written.out).expect("utf-8");
        assert_eq!(text.lines().count(), 2);
        assert!(text.ends_with("}\n"));

        let mut failing = RecordWriter::new(Refusing, PathBuf::from("record.jsonl"));
        let error = failing.append(&line).expect_err("the write fails");
        assert!(
            matches!(error, Error::Record { ref path, .. } if path == &PathBuf::from("record.jsonl"))
        );
    }

    #[test]
    fn a_nullable_field_may_not_be_omitted() {
        let mut line = replicate_of(&MetricEntry::from_series(&[1.0]).expect("entry"));
        line.as_object_mut().expect("object").remove("failure");
        let text = record(&[started(true), canary("start"), line]);
        let error = validate(&text).expect_err("failure is required");
        assert!(
            matches!(error, Error::InvalidRecord { line: 3, ref reason } if reason.contains("failure"))
        );
    }

    #[test]
    fn a_host_value_object_carries_exactly_one_key() {
        let mut line = started(true);
        line["host"]["smt"] = json!({ "absent": "no source", "extra": "x" });
        assert!(validate(&record(&[line])).is_err());
    }

    #[test]
    fn a_killed_run_is_a_valid_prefix() {
        for lines in [
            vec![started(true)],
            vec![started(true), canary("start")],
            vec![started(true), canary("start"), canary("end")],
        ] {
            assert!(validate(&record(&lines)).is_ok());
        }
    }

    #[test]
    fn the_line_order_is_enforced() {
        let entry = MetricEntry::from_series(&[1.0]).expect("entry");
        let cases = [
            (vec![started(true), replicate_of(&entry)], 2),
            (vec![started(true), canary("end")], 2),
            (vec![started(true), canary("start"), canary("start")], 3),
            (vec![started(true), canary("start"), finished()], 3),
            (vec![started(true), canary("start"), canary("end"), replicate_of(&entry)], 4),
            (vec![started(true), started(true)], 2),
            (
                vec![
                    started(true),
                    canary("start"),
                    json!({"t":"run_refused","setting":"s","reason":"r"}),
                ],
                3,
            ),
        ];
        for (lines, at) in cases {
            let error = validate(&record(&lines)).expect_err("out of order");
            assert!(
                matches!(error, Error::InvalidRecord { line, .. } if line == at),
                "{lines:?}: {error}"
            );
        }
    }

    #[test]
    fn a_refusal_carries_a_difference_or_a_reason_and_nothing_else() {
        for (refusal, valid) in [
            (json!({"t":"run_refused","setting":"s","found":"a","pinned":"b"}), true),
            (json!({"t":"run_refused","setting":"s","reason":"r"}), true),
            (json!({"t":"run_refused","setting":"s"}), false),
            (json!({"t":"run_refused","setting":"s","found":"a"}), false),
            (json!({"t":"run_refused","setting":"s","pinned":"b"}), false),
            (json!({"t":"run_refused","setting":"s","reason":"r","found":"a","pinned":"b"}), false),
            (json!({"t":"run_refused","setting":"s","reason":"r","found":"a"}), false),
        ] {
            let result = validate(&record(&[started(true), refusal.clone()]));
            assert_eq!(result.is_ok(), valid, "{refusal}");
        }
    }

    #[test]
    fn a_metric_entry_in_a_failed_replicate_may_be_partial() {
        let mut line = replicate_of(&MetricEntry::from_series(&[1.0]).expect("entry"));
        line["outcome"] = json!("failed");
        line["failure"] = json!("fixture broke");
        line["metrics"] =
            json!({ "m": { "values": [], "histogram": null, "failures": [], "absent": null } });
        let failed = json!({ "t": "run_finished", "finished": "x",
                             "replicates_ok": 0, "replicates_failed": 1 });
        let text = record(&[started(true), canary("start"), line, canary("end"), failed]);
        assert!(validate(&text).is_ok());
    }

    proptest! {
        #[test]
        fn what_the_writer_stores_validates_at_any_series_length(len in 1usize..2200) {
            let values: Vec<f64> = (0..len).map(|i| (i % 9000) as f64 * 1.37).collect();
            let entry = MetricEntry::from_series(&values).expect("storable");
            prop_assert_eq!(entry.histogram.is_some(), len > RAW_VALUE_LIMIT);
            prop_assert_eq!(entry.values.len(), if len > RAW_VALUE_LIMIT { 0 } else { len });
            prop_assert!(validate(&series_record(&entry)).is_ok());
        }

        #[test]
        fn a_stored_entry_survives_a_write_and_read(
            quarters in proptest::collection::vec(0u32..4_000_000, 1..1300),
        ) {
            // Quarter-integers are exact in binary and in decimal, so the round trip is exact.
            let values: Vec<f64> = quarters.iter().map(|q| f64::from(*q) / 4.0).collect();
            let entry = MetricEntry::from_series(&values).expect("storable");
            let text = serde_json::to_string(&entry).expect("serialise");
            let back: MetricEntry = serde_json::from_str(&text).expect("parse");
            prop_assert_eq!(back, entry);
        }
    }

    // ---- `GuestMemoryMib` (guest-benchmark-substrate, DDD-4) ----
    //
    // A positive MiB count: `Display` is the decimal digits, `FromStr` accepts decimal digits
    // only and refuses zero and anything else with a typed error, serde is the JSON integer
    // the record carries and deserialising 0 is refused naming `memory_mib`.
    // CONTRACT_SHAPE: pure-function. Layers 1-2: PBT full (Mandate 9).

    fn guest_memory() -> impl Strategy<Value = GuestMemoryMib> {
        (1u32..=u32::MAX).prop_map(|mib| {
            GuestMemoryMib::new(std::num::NonZeroU32::new(mib).expect("a positive count"))
        })
    }

    /// `Display` is the decimal digits of the count.
    #[test]
    fn a_guest_memory_size_displays_as_its_decimal_digits() {
        let mib = GuestMemoryMib::new(std::num::NonZeroU32::new(512).expect("512"));
        assert_eq!(mib.to_string(), "512");
    }

    proptest! {
        #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

        /// Text round trip: `Display` then `FromStr` returns the size.
        #[test]
        fn a_guest_memory_size_round_trips_through_its_text(size in guest_memory()) {
            let text = size.to_string();
            prop_assert_eq!(
                text.parse::<GuestMemoryMib>(),
                Ok(size),
                "the text of a size must parse back to the size"
            );
        }

        /// Record round trip: the JSON integer is the same digits as `Display`, and it reads back.
        #[test]
        fn a_guest_memory_size_round_trips_through_the_json_integer_the_record_carries(
            size in guest_memory(),
        ) {
            let json = serde_json::to_string(&size).expect("serialise");
            prop_assert_eq!(
                &json,
                &size.to_string(),
                "the record carries the size as the JSON integer of its digits"
            );
            let back: GuestMemoryMib = serde_json::from_str(&json).expect("parse");
            prop_assert_eq!(back, size);
        }
    }

    /// Text that is not decimal digits, or is zero, or overflows `u32`, is refused with a typed
    /// error; each case is one named sad path (the boundary 0 and `u32::MAX + 1` are listed).
    #[test]
    fn text_that_is_not_a_positive_decimal_count_is_refused() {
        // Control first: a refusal table is vacuous against a parser that refuses everything.
        assert_eq!(
            "4294967295".parse::<GuestMemoryMib>().map(|mib| mib.to_string()),
            Ok("4294967295".to_owned()),
            "the largest count parses"
        );
        for text in ["", "0", "00", "-1", "+5", "1.5", "5 ", " 5", "0x10", "512mib", "4294967296"] {
            assert!(text.parse::<GuestMemoryMib>().is_err(), "`{text}` is not a guest memory size");
        }
    }

    /// Deserialising zero, a negative number, a fraction or a string is refused, and the refusal
    /// of zero names `memory_mib`.
    #[test]
    fn a_json_value_that_is_not_a_positive_integer_is_refused_naming_the_field() {
        // Control first: a refusal table is vacuous against a deserialiser that refuses everything.
        assert_eq!(
            serde_json::from_str::<GuestMemoryMib>("512").map(|mib| mib.to_string()).ok(),
            Some("512".to_owned()),
            "a positive JSON integer deserialises to its size"
        );
        let zero = serde_json::from_str::<GuestMemoryMib>("0").expect_err("zero is refused");
        assert!(zero.to_string().contains("memory_mib"), "the refusal names memory_mib: {zero}");
        for text in ["-1", "1.5", "\"512\"", "null", "4294967296"] {
            assert!(serde_json::from_str::<GuestMemoryMib>(text).is_err(), "`{text}` is refused");
        }
    }
}
