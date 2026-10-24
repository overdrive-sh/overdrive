//! ITF decoding helpers shared by the guest flow owner's quint-connect
//! conformance drivers (`quint_owner_flows`, `quint_fwd_pair`,
//! `quint_udp_slots`; QPROJ-1; D-UP-1, spike QT1).
//!
//! **Qualified names (QT1 caveat 1).** A trace state is an ITF record keyed by
//! the spec's qualified variable names, `<main>::<module>::<var>`
//! (`flows_conf::owner_flows::gf`). A driver names its conformance main and the
//! module that declares the variables once, as a [`Qualified`] scope, and reads
//! each variable through it; a key with another prefix is not one of the
//! scope's variables, so a driver pointed at the wrong main (a non-recording
//! one decodes every step as its initial action, QT1) fails the projection with
//! [`ItfError::MissingVar`] naming the qualified variable rather than comparing
//! the wrong instance. The ITF metadata (`#meta`) and quint-connect's `mbt::*`
//! bookkeeping are not variables and are ignored.
//!
//! **Values.** Quint integers arrive as `#bigint` (or plain numbers), maps as
//! `#map`, sets as `#set`, tuples as `#tup`, lists as JSON arrays, records as
//! objects and sum types as `{ "tag": .., "value": .. }`. The drivers decode the
//! variables they compare straight from [`itf::Value`], so no serde shape of
//! the trace is assumed beyond the ITF value model.

#![allow(
    dead_code,
    reason = "a shared decoder: each driver uses the subset its module's variables need"
)]

use std::collections::{BTreeMap, BTreeSet};

use itf::Value;

/// Why a trace value does not have the shape a driver compares.
#[derive(Debug, thiserror::Error)]
pub(super) enum ItfError {
    /// The state carries no variable of that qualified name.
    #[error("the trace state has no variable `{0}`")]
    MissingVar(String),
    /// A record lacks a field.
    #[error("the trace's record has no `{0}` field")]
    MissingField(String),
    /// A value has another shape than the driver decodes.
    #[error("expected {expected} in the trace, found {found:?}")]
    Shape {
        /// What the driver decodes.
        expected: &'static str,
        /// What the trace holds.
        found: Value,
    },
}

fn shape(expected: &'static str, found: &Value) -> ItfError {
    ItfError::Shape { expected, found: found.clone() }
}

/// The variables of one conformance main's module in one trace state, read
/// by their qualified names `<main>::<module>::<var>`.
#[derive(Debug, Clone)]
pub(super) struct Qualified {
    prefix: String,
    vars: BTreeMap<String, Value>,
}

impl Qualified {
    /// The variables of `state` declared by `module` on the conformance main
    /// `main` (for example `Qualified::of(state, "flows_conf", "owner_flows")`).
    /// Every other key is ignored; a state that is not a record is a shape
    /// error.
    pub(super) fn of(state: &Value, main: &str, module: &str) -> Result<Self, ItfError> {
        let Value::Record(record) = state else {
            return Err(shape("a record of variables", state));
        };
        let prefix = format!("{main}::{module}::");
        let vars = record
            .iter()
            .filter(|(key, _)| key.starts_with(&prefix))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        Ok(Self { prefix, vars })
    }

    /// Variable `name` (unqualified), looked up as `<main>::<module>::<name>`.
    pub(super) fn var(&self, name: &str) -> Result<&Value, ItfError> {
        let qualified = format!("{}{name}", self.prefix);
        self.vars.get(&qualified).ok_or(ItfError::MissingVar(qualified))
    }

    /// The scope's qualified-name prefix, `<main>::<module>::`.
    pub(super) fn prefix(&self) -> &str {
        &self.prefix
    }
}

/// Field `name` of a Quint record.
pub(super) fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, ItfError> {
    let Value::Record(record) = value else { return Err(shape("a record", value)) };
    record.get(name).ok_or_else(|| ItfError::MissingField(name.to_owned()))
}

/// A Quint `int`.
pub(super) fn int(value: &Value) -> Result<i64, ItfError> {
    match value {
        Value::Number(n) => Ok(*n),
        Value::BigInt(big) => {
            i64::try_from(big.get().clone()).map_err(|_| shape("a 64-bit int", value))
        }
        other => Err(shape("an int", other)),
    }
}

/// A Quint `bool`.
pub(super) fn boolean(value: &Value) -> Result<bool, ItfError> {
    match value {
        Value::Bool(b) => Ok(*b),
        other => Err(shape("a bool", other)),
    }
}

/// A Quint `str`.
pub(super) fn string(value: &Value) -> Result<String, ItfError> {
    match value {
        Value::String(s) => Ok(s.clone()),
        other => Err(shape("a string", other)),
    }
}

/// The tag of a Quint sum-type value (`variant("X", ..)` → `"X"`).
pub(super) fn tag(value: &Value) -> Result<String, ItfError> {
    string(field(value, "tag")?)
}

/// The value of a Quint sum-type value.
pub(super) fn tag_value(value: &Value) -> Result<&Value, ItfError> {
    field(value, "value")
}

/// The entries of a Quint map.
pub(super) fn map_entries(value: &Value) -> Result<Vec<(&Value, &Value)>, ItfError> {
    match value {
        Value::Map(map) => Ok(map.iter().collect()),
        other => Err(shape("a map", other)),
    }
}

/// The elements of a Quint set.
pub(super) fn set_elems(value: &Value) -> Result<Vec<&Value>, ItfError> {
    match value {
        Value::Set(set) => Ok(set.iter().collect()),
        // Quint's ITF encoder cannot distinguish an empty set from an empty
        // sequence once the value has no element witness. The caller supplies
        // the set type by choosing this decoder.
        Value::List(list) if list.is_empty() => Ok(Vec::new()),
        other => Err(shape("a set", other)),
    }
}

/// The elements of a Quint list.
pub(super) fn list_elems(value: &Value) -> Result<Vec<&Value>, ItfError> {
    match value {
        Value::List(list) => Ok(list.iter().collect()),
        other => Err(shape("a list", other)),
    }
}

/// The components of a Quint tuple.
pub(super) fn tuple_elems(value: &Value) -> Result<Vec<&Value>, ItfError> {
    match value {
        Value::Tuple(tuple) => Ok(tuple.iter().collect()),
        other => Err(shape("a tuple", other)),
    }
}

/// A Quint `Set[int]`.
pub(super) fn int_set(value: &Value) -> Result<BTreeSet<i64>, ItfError> {
    set_elems(value)?.into_iter().map(int).collect()
}

/// A Quint `Set[str]`.
pub(super) fn str_set(value: &Value) -> Result<BTreeSet<String>, ItfError> {
    set_elems(value)?.into_iter().map(string).collect()
}

/// A Quint `str -> T` map, decoded per entry.
pub(super) fn str_map<T>(
    value: &Value,
    decode: impl Fn(&Value) -> Result<T, ItfError>,
) -> Result<BTreeMap<String, T>, ItfError> {
    map_entries(value)?.into_iter().map(|(k, v)| Ok((string(k)?, decode(v)?))).collect()
}

/// A Quint `int -> T` map, decoded per entry.
pub(super) fn int_map<T>(
    value: &Value,
    decode: impl Fn(&Value) -> Result<T, ItfError>,
) -> Result<BTreeMap<i64, T>, ItfError> {
    map_entries(value)?.into_iter().map(|(k, v)| Ok((int(k)?, decode(v)?))).collect()
}
