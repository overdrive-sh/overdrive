//! ITF decoding helpers shared by the guest flow owner's quint-connect
//! conformance drivers (`quint_owner_flows`, `quint_flows_half`,
//! `quint_udp_slots`).
//!
//! A trace state is an ITF record whose variables are named
//! `<instance>::<module>::<var>`; Quint integers arrive as `#bigint` (or plain
//! numbers), maps as `#map`, sets as `#set`, tuples as `#tup`, lists as JSON
//! arrays. The drivers decode the variables they compare straight from
//! [`itf::Value`] (their `State::from_spec`), so no serde shape of the trace is
//! assumed beyond the ITF value model.

use std::collections::{BTreeMap, BTreeSet};

use itf::Value;

/// Why a trace value does not have the shape a driver compares.
#[derive(Debug, thiserror::Error)]
pub(super) enum ItfError {
    #[error("the trace has no `{0}` variable")]
    MissingVar(String),
    #[error("the trace's record has no `{0}` field")]
    MissingField(String),
    #[error("expected {expected} in the trace, found {found:?}")]
    Shape { expected: &'static str, found: Value },
}

fn shape(expected: &'static str, found: &Value) -> ItfError {
    ItfError::Shape { expected, found: found.clone() }
}

/// The trace state's variables, keyed by their own name (the
/// `<instance>::<module>::` prefix stripped).
pub(super) fn vars(state: &Value) -> Result<BTreeMap<String, Value>, ItfError> {
    let Value::Record(record) = state else { return Err(shape("a record of variables", state)) };
    Ok(record
        .iter()
        .map(|(key, value)| (key.rsplit("::").next().unwrap_or(key).to_owned(), value.clone()))
        .collect())
}

/// Variable `name` of the stripped variable map.
pub(super) fn var<'a>(
    vars: &'a BTreeMap<String, Value>,
    name: &str,
) -> Result<&'a Value, ItfError> {
    vars.get(name).ok_or_else(|| ItfError::MissingVar(name.to_owned()))
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
