//! Counterexample traces: read an Apalache ITF trace or a TLC text trace
//! and condense it into a short state sequence for `evidence/traces/`.
//!
//! The condensed form prints the first state in full and, for every later
//! state, only the variables whose value changed — the part a reviewer reads
//! to follow a counterexample. Variable names lose the instance prefix the
//! tools add (`main::module::` in ITF, `<main>_<module>_` in TLC).

use std::fmt::Write as _;

use serde_json::Value;

/// A counterexample trace, independent of the tool that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Trace {
    /// States in order.
    pub states: Vec<TraceState>,
    /// For a lasso (liveness counterexample): the 1-based state the last
    /// state loops back to.
    pub loop_back: Option<usize>,
    /// For a liveness counterexample that ends by stuttering (TLC's
    /// `State N: Stuttering`): the last state repeats forever.
    pub stutters: bool,
}

/// One state of a [`Trace`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TraceState {
    /// The action that produced the state, when the tool names it.
    pub action: Option<String>,
    /// `(variable, rendered value)` pairs, in the tool's order.
    pub vars: Vec<(String, String)>,
}

/// Errors reading a trace.
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    /// The ITF file is not JSON.
    #[error("ITF trace is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// The JSON does not have the ITF shape.
    #[error("ITF trace: {0}")]
    Shape(&'static str),
    /// The log holds neither a TLC state sequence (`State 1:` …) nor an
    /// initial-state violation (`Error: … is violated by the initial state:`).
    #[error(
        "no TLC counterexample in the log: neither `State 1:` nor \
         `Error: … is violated by the initial state:` found"
    )]
    NoTlcTrace,
    /// A variable assignment appears before any state header.
    #[error("TLC trace: assignment before any state header: `{line}`")]
    AssignmentOutsideState {
        /// The offending line.
        line: String,
    },
}

/// Parse an ITF trace (Apalache `--out-itf`).
pub fn parse_itf(body: &str) -> Result<Trace, TraceError> {
    let doc: Value = serde_json::from_str(body)?;
    let vars: Vec<&str> = doc
        .get("vars")
        .and_then(Value::as_array)
        .ok_or(TraceError::Shape("missing `vars` array"))?
        .iter()
        .map(|v| v.as_str().ok_or(TraceError::Shape("non-string entry in `vars`")))
        .collect::<Result<_, _>>()?;
    let states = doc
        .get("states")
        .and_then(Value::as_array)
        .ok_or(TraceError::Shape("missing `states` array"))?;
    let mut out = Trace {
        states: Vec::with_capacity(states.len()),
        loop_back: doc
            .get("loop")
            .and_then(Value::as_u64)
            .and_then(|l| usize::try_from(l).ok())
            .map(|l| l + 1),
        stutters: false,
    };
    for state in states {
        let obj = state.as_object().ok_or(TraceError::Shape("state is not an object"))?;
        let mut vars_out = Vec::with_capacity(vars.len());
        for name in &vars {
            let value =
                obj.get(*name).ok_or(TraceError::Shape("state lacks a declared variable"))?;
            vars_out.push((short_itf_name(name).to_owned(), render_itf(value)));
        }
        out.states.push(TraceState { action: None, vars: vars_out });
    }
    Ok(out)
}

/// `main::module::var` → `var`.
fn short_itf_name(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

/// Render an ITF value in Quint-like syntax.
pub fn render_itf(v: &Value) -> String {
    match v {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("{s:?}"),
        Value::Array(items) => format!("List({})", join(items.iter().map(render_itf))),
        Value::Object(obj) => {
            if let Some(n) = obj.get("#bigint").and_then(Value::as_str) {
                return n.to_owned();
            }
            if let Some(items) = obj.get("#tup").and_then(Value::as_array) {
                return format!("({})", join(items.iter().map(render_itf)));
            }
            if let Some(items) = obj.get("#set").and_then(Value::as_array) {
                return format!("Set({})", join(items.iter().map(render_itf)));
            }
            if let Some(pairs) = obj.get("#map").and_then(Value::as_array) {
                return format!(
                    "Map({})",
                    join(pairs.iter().map(|p| match p.as_array().map(Vec::as_slice) {
                        Some([k, v]) => format!("{} -> {}", render_itf(k), render_itf(v)),
                        _ => render_itf(p),
                    }))
                );
            }
            if let Some(s) = obj.get("#unserializable").and_then(Value::as_str) {
                return s.to_owned();
            }
            if obj.len() == 2
                && let (Some(Value::String(tag)), Some(value)) = (obj.get("tag"), obj.get("value"))
            {
                let unit = value.get("#tup").and_then(Value::as_array).is_some_and(Vec::is_empty);
                return if unit { tag.clone() } else { format!("{tag}({})", render_itf(value)) };
            }
            format!("{{ {} }}", join(obj.iter().map(|(k, v)| format!("{k}: {}", render_itf(v)))))
        }
    }
}

fn join(parts: impl Iterator<Item = String>) -> String {
    parts.collect::<Vec<_>>().join(", ")
}

/// Extract TLC's counterexample text from a `quint verify --backend tlc
/// --verbosity=3` log: from the first state header to the end of the state
/// sequence, in TLC's own words.
///
/// TLC 2.19 (via Quint 0.32) prints a counterexample in one of these shapes
/// (fixtures under `testdata/tlc/`, captured from real runs):
///
/// - **multi-state** (invariant violated after ≥1 step):
///   `Error: The behavior up to this point is:` then `State 1: <Initial
///   predicate>`, `State 2: <action …>`, …;
/// - **lasso** (temporal property): the same, ending in `Back to state N: …`;
/// - **stuttering** (temporal property): the same, ending in `State N:
///   Stuttering`;
/// - **initial state** (invariant — or an `always(P)` temporal property,
///   which TLC checks as an invariant — violated by an initial state):
///   `Error: Invariant <name> is violated by the initial state:` followed
///   directly by the state's variables, with no `State 1:` header.
///
/// Variables are printed as `/\ name = value` when the spec has several
/// and as a bare `name = value` when it has one; long values wrap onto
/// indented continuation lines. Quint runs TLC without deadlock checking,
/// so a deadlock is never reported as a counterexample (`testdata/tlc/
/// deadlock.log`: a reachable state with no successor, `[ok]`).
pub fn extract_tlc_trace(log: &str) -> Result<String, TraceError> {
    let mut lines = log.lines().skip_while(|l| !is_state_header(l) && !is_initial_violation(l));
    let first = lines.next().ok_or(TraceError::NoTlcTrace)?;
    let mut out = String::from(first);
    out.push('\n');
    for line in lines {
        let keep = line.is_empty()
            || is_state_header(line)
            || line.starts_with("/\\ ")
            || bare_assignment(line).is_some()
            || line.starts_with(char::is_whitespace)
            || line.starts_with("Back to state");
        if !keep {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    Ok(out.trim_end().to_owned() + "\n")
}

fn is_state_header(line: &str) -> bool {
    line.strip_prefix("State ")
        .and_then(|r| r.split_once(':'))
        .is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// `Error: Invariant <name> is violated by the initial state:`.
fn is_initial_violation(line: &str) -> bool {
    line.starts_with("Error: ") && line.trim_end().ends_with("is violated by the initial state:")
}

/// A single-variable spec's `name = value` line (TLC drops the `/\` when
/// there is only one variable): `(name, value)`.
fn bare_assignment(line: &str) -> Option<(&str, &str)> {
    let (name, value) = line.split_once(" = ")?;
    let ident = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    ident.then_some((name, value))
}

/// Parse TLC's counterexample text (see [`extract_tlc_trace`]).
pub fn parse_tlc(text: &str) -> Result<Trace, TraceError> {
    let mut trace = Trace::default();
    for line in text.lines() {
        if is_initial_violation(line) {
            trace.states.push(TraceState::default());
        } else if is_state_header(line) {
            let rest = line.split_once(':').map_or("", |(_, r)| r).trim();
            if let Some(n) = back_to(rest) {
                trace.loop_back = Some(n);
                continue;
            }
            if rest == "Stuttering" {
                trace.stutters = true;
                continue;
            }
            let action = rest
                .strip_prefix('<')
                .and_then(|r| r.strip_suffix('>'))
                .unwrap_or(rest)
                .split(" line ")
                .next()
                .unwrap_or_default()
                .trim();
            trace.states.push(TraceState {
                action: (!action.is_empty() && action != "Initial predicate")
                    .then(|| action.to_owned()),
                vars: Vec::new(),
            });
        } else if let Some(n) = back_to(line) {
            trace.loop_back = Some(n);
        } else if let Some((name, value)) = line
            .strip_prefix("/\\ ")
            .map(|a| a.split_once(" = ").unwrap_or((a, "")))
            .or_else(|| bare_assignment(line))
        {
            let state = trace
                .states
                .last_mut()
                .ok_or_else(|| TraceError::AssignmentOutsideState { line: line.to_owned() })?;
            state.vars.push((name.trim().to_owned(), value.trim().to_owned()));
        } else if line.starts_with(char::is_whitespace) && !line.trim().is_empty() {
            // TLC wraps long values onto indented continuation lines.
            if let Some((_, value)) = trace.states.last_mut().and_then(|s| s.vars.last_mut()) {
                value.push(' ');
                value.push_str(line.trim());
            }
        }
    }
    if trace.states.is_empty() {
        return Err(TraceError::NoTlcTrace);
    }
    strip_tlc_prefix(&mut trace);
    Ok(trace)
}

/// `Back to state 3` / `Back to state: 3` → `3`.
fn back_to(s: &str) -> Option<usize> {
    let rest = s.strip_prefix("Back to state")?.trim_start_matches([':', ' ']);
    rest.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

/// Drop the `<main>_<module>_` prefix TLC variable and action names share.
fn strip_tlc_prefix(trace: &mut Trace) {
    let names: Vec<&str> =
        trace.states.iter().flat_map(|s| s.vars.iter().map(|(n, _)| n.as_str())).collect();
    let Some(first) = names.first() else { return };
    let mut common = first.len();
    for n in &names {
        common = common.min(first.bytes().zip(n.bytes()).take_while(|(a, b)| a == b).count());
    }
    let Some(prefix_len) = first[..common].rfind('_').map(|i| i + 1) else { return };
    let distinct: std::collections::BTreeSet<&str> = names.iter().copied().collect();
    if distinct.len() < 2 || names.iter().any(|n| n.len() <= prefix_len) {
        return;
    }
    let prefix = first[..prefix_len].to_owned();
    for state in &mut trace.states {
        for (name, _) in &mut state.vars {
            *name = name[prefix_len..].to_owned();
        }
        if let Some(action) = &mut state.action
            && let Some(short) = action.strip_prefix(&prefix)
        {
            *action = short.to_owned();
        }
    }
}

/// Condense a trace: the first state in full, then per state only the
/// variables whose value changed.
pub fn condense(check: &str, source: &str, trace: &Trace) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# {check}: counterexample, {} state(s) (from the {source} trace)",
        trace.states.len()
    );
    let mut prev: Vec<(String, String)> = Vec::new();
    for (i, state) in trace.states.iter().enumerate() {
        let _ = write!(out, "\nState {}", i + 1);
        if let Some(action) = &state.action {
            let _ = write!(out, " [{action}]");
        }
        out.push('\n');
        let changed: Vec<&(String, String)> = state
            .vars
            .iter()
            .filter(|(name, value)| {
                i == 0 || prev.iter().find(|(n, _)| n == name).is_none_or(|(_, v)| v != value)
            })
            .collect();
        if changed.is_empty() {
            out.push_str("  (no change)\n");
        }
        for (name, value) in changed {
            let _ = writeln!(out, "  {name} = {value}");
        }
        prev.clone_from(&state.vars);
    }
    if let Some(n) = trace.loop_back {
        let _ = writeln!(out, "\n-> loops back to State {n}");
    }
    if trace.stutters {
        let _ = writeln!(out, "\n-> stutters forever in State {}", trace.states.len());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ITF: &str = r##"{
      "#meta": {"format": "ITF"},
      "vars": ["m::mod::gen", "m::mod::flows", "m::mod::st", "m::mod::seen"],
      "states": [
        {"#meta": {"index": 0}, "m::mod::gen": {"#bigint": "0"},
         "m::mod::flows": {"#map": []}, "m::mod::st": {"tag": "Idle", "value": {"#tup": []}},
         "m::mod::seen": {"#set": []}},
        {"#meta": {"index": 1}, "m::mod::gen": {"#bigint": "1"},
         "m::mod::flows": {"#map": [[{"#bigint": "3"}, {"gen": {"#bigint": "1"}, "st": "open"}]]},
         "m::mod::st": {"tag": "Busy", "value": {"#bigint": "7"}},
         "m::mod::seen": {"#set": []}},
        {"#meta": {"index": 2}, "m::mod::gen": {"#bigint": "1"},
         "m::mod::flows": {"#map": [[{"#bigint": "3"}, {"gen": {"#bigint": "1"}, "st": "open"}]]},
         "m::mod::st": {"tag": "Busy", "value": {"#bigint": "7"}},
         "m::mod::seen": {"#set": [{"#tup": [{"#bigint": "1"}, true]}]}}
      ]
    }"##;

    #[test]
    fn itf_condenses_to_changed_variables_with_short_names() {
        let trace = parse_itf(ITF).expect("valid ITF");
        assert_eq!(trace.states.len(), 3);
        assert_eq!(
            condense("c", "ITF", &trace),
            "# c: counterexample, 3 state(s) (from the ITF trace)\n\
             \nState 1\n  gen = 0\n  flows = Map()\n  st = Idle\n  seen = Set()\n\
             \nState 2\n  gen = 1\n  flows = Map(3 -> { gen: 1, st: \"open\" })\n  st = Busy(7)\n\
             \nState 3\n  seen = Set((1, true))\n"
        );
    }

    #[test]
    fn itf_rejects_wrong_shape() {
        assert!(matches!(parse_itf("[]"), Err(TraceError::Shape(_))));
        assert!(matches!(parse_itf("{"), Err(TraceError::Json(_))));
        assert!(matches!(
            parse_itf(r#"{"vars":["x"],"states":[{"y":1}]}"#),
            Err(TraceError::Shape(_))
        ));
    }

    // Captured from `quint verify --backend tlc --verbosity=3` (Quint 0.32 / TLC 2.19),
    // with a wrapped continuation line, a no-change state and a lasso added.
    const TLC_LOG: &str = "Finished computing initial states: 1 distinct state generated.\n\
Error: Invariant q_inv is violated.\n\
Error: The behavior up to this point is:\n\
State 1: <Initial predicate>\n\
/\\ lt_cid_lease_al = <<[st |-> \"wait\", off |-> -1]>>\n\
/\\ lt_cid_lease_lease = (0 :> 0 @@ 1 :> 0)\n\
/\\ lt_cid_lease_cursor = 0\n\
\n\
State 2: <lt_cid_lease_Assign line 495, col 3 to line 570, col 44 of module lt>\n\
/\\ lt_cid_lease_al = <<[st |-> \"held\",\n\
\x20    off |-> 0]>>\n\
/\\ lt_cid_lease_lease = (0 :> 1 @@ 1 :> 0)\n\
/\\ lt_cid_lease_cursor = 1\n\
\n\
State 3: <lt_cid_lease_Tick line 9, col 3 to line 9, col 9 of module lt>\n\
/\\ lt_cid_lease_al = <<[st |-> \"held\", off |-> 0]>>\n\
/\\ lt_cid_lease_lease = (0 :> 1 @@ 1 :> 0)\n\
/\\ lt_cid_lease_cursor = 1\n\
\n\
Back to state 2: <lt_cid_lease_Tick line 9, col 3 to line 9, col 9 of module lt>\n\
\n\
65 states generated, 61 distinct states found, 1 states left on queue.\n\
[violation] Found an issue (811ms).\n";

    #[test]
    fn tlc_trace_is_extracted_and_condensed() {
        let text = extract_tlc_trace(TLC_LOG).expect("trace present");
        assert!(text.starts_with("State 1: <Initial predicate>\n"), "{text}");
        assert!(
            text.ends_with(
                "Back to state 2: <lt_cid_lease_Tick line 9, col 3 to line 9, col 9 of module lt>\n"
            ),
            "{text}"
        );
        let trace = parse_tlc(&text).expect("parses");
        assert_eq!(
            condense("c", "TLC", &trace),
            "# c: counterexample, 3 state(s) (from the TLC trace)\n\
             \nState 1\n  al = <<[st |-> \"wait\", off |-> -1]>>\n  lease = (0 :> 0 @@ 1 :> 0)\n  cursor = 0\n\
             \nState 2 [Assign]\n  al = <<[st |-> \"held\", off |-> 0]>>\n  lease = (0 :> 1 @@ 1 :> 0)\n  cursor = 1\n\
             \nState 3 [Tick]\n  (no change)\n\
             \n-> loops back to State 2\n"
        );
    }

    #[test]
    fn tlc_without_trace_is_an_error() {
        assert!(matches!(
            extract_tlc_trace("[ok] No violation found (1ms).\n"),
            Err(TraceError::NoTlcTrace)
        ));
        assert!(matches!(parse_tlc("nothing here\n"), Err(TraceError::NoTlcTrace)));
    }

    /// A real `quint verify --backend tlc --verbosity=3` log, captured in the
    /// Lima VM (Quint 0.32.0, TLC 2.19) from the specs beside it in
    /// `testdata/tlc/` (`shapes.qnt`: one variable; `multi.qnt`: two, one of
    /// them a set long enough for TLC to wrap).
    fn fixture(name: &str) -> &'static str {
        match name {
            "init-inv" => include_str!("testdata/tlc/init-inv.log"),
            "init-temporal" => include_str!("testdata/tlc/init-temporal.log"),
            "multi-inv" => include_str!("testdata/tlc/multi-inv.log"),
            "lasso" => include_str!("testdata/tlc/lasso.log"),
            "stutter" => include_str!("testdata/tlc/stutter.log"),
            "deadlock" => include_str!("testdata/tlc/deadlock.log"),
            "multivar-init-inv" => include_str!("testdata/tlc/multivar-init-inv.log"),
            "multivar-multi-inv" => include_str!("testdata/tlc/multivar-multi-inv.log"),
            "multivar-lasso" => include_str!("testdata/tlc/multivar-lasso.log"),
            other => unreachable!("no fixture {other}"),
        }
    }

    fn condensed(name: &str) -> String {
        let text = extract_tlc_trace(fixture(name)).expect("trace present");
        condense("c", "TLC", &parse_tlc(&text).expect("parses"))
    }

    const BIG: &str = "{ [name |-> \"allocation-number\", offset |-> 0, state |-> \"waiting-for-device\"], \
        [name |-> \"allocation-number\", offset |-> 1, state |-> \"waiting-for-device\"], \
        [name |-> \"allocation-number\", offset |-> 2, state |-> \"waiting-for-device\"], \
        [name |-> \"allocation-number\", offset |-> 3, state |-> \"waiting-for-device\"], \
        [name |-> \"allocation-number\", offset |-> 4, state |-> \"waiting-for-device\"], \
        [name |-> \"allocation-number\", offset |-> 5, state |-> \"waiting-for-device\"], \
        [name |-> \"allocation-number\", offset |-> 6, state |-> \"waiting-for-device\"] }";

    #[test]
    fn tlc_initial_state_violation_is_a_one_state_trace() {
        // No `State 1:` header: TLC names the violated invariant and prints
        // the initial state's variables directly beneath it.
        let text = extract_tlc_trace(fixture("init-inv")).expect("trace present");
        assert_eq!(text, "Error: Invariant q_inv is violated by the initial state:\nx = 0\n");
        assert_eq!(
            condensed("init-inv"),
            "# c: counterexample, 1 state(s) (from the TLC trace)\n\nState 1\n  x = 0\n"
        );
    }

    #[test]
    fn tlc_always_property_violated_initially_is_a_one_state_trace() {
        // TLC checks `always(P)` as an invariant, so a `temporal` check can
        // end in the initial-state shape too.
        assert_eq!(
            condensed("init-temporal"),
            "# c: counterexample, 1 state(s) (from the TLC trace)\n\nState 1\n  x = 0\n"
        );
    }

    #[test]
    fn tlc_multi_variable_initial_state_violation_keeps_wrapped_values() {
        assert_eq!(
            condensed("multivar-init-inv"),
            format!(
                "# c: counterexample, 1 state(s) (from the TLC trace)\n\nState 1\n  x = 0\n  \
                 big = {BIG}\n"
            )
        );
    }

    #[test]
    fn tlc_multi_state_violation_lists_every_state() {
        assert_eq!(
            condensed("multi-inv"),
            "# c: counterexample, 3 state(s) (from the TLC trace)\n\
             \nState 1\n  x = 0\n\nState 2 [:=]\n  x = 1\n\nState 3 [:=]\n  x = 2\n"
        );
        assert_eq!(
            condensed("multivar-multi-inv"),
            format!(
                "# c: counterexample, 3 state(s) (from the TLC trace)\n\
                 \nState 1\n  x = 0\n  big = {BIG}\n\nState 2 [step]\n  x = 1\n\
                 \nState 3 [step]\n  x = 2\n"
            )
        );
    }

    #[test]
    fn tlc_lasso_records_the_loop_back() {
        let lasso = "# c: counterexample, 3 state(s) (from the TLC trace)\n\
             \nState 1\n  x = 0\n\nState 2 [:=]\n  x = 1\n\nState 3 [:=]\n  x = 2\n\
             \n-> loops back to State 1\n";
        assert_eq!(condensed("lasso"), lasso);
        assert!(condensed("multivar-lasso").ends_with("\n-> loops back to State 1\n"));
    }

    #[test]
    fn tlc_stuttering_ends_the_trace_without_a_fake_state() {
        assert_eq!(
            condensed("stutter"),
            "# c: counterexample, 2 state(s) (from the TLC trace)\n\
             \nState 1\n  x = 0\n\nState 2 [:=]\n  x = 1\n\
             \n-> stutters forever in State 2\n"
        );
    }

    #[test]
    fn tlc_deadlock_is_not_reported_by_quint() {
        // Quint runs TLC without deadlock checking: `dl` reaches `x = 2`,
        // which has no successor, and the run still ends `[ok]`.
        let log = fixture("deadlock");
        assert!(log.contains("[ok] No violation found"), "{log}");
        assert!(matches!(extract_tlc_trace(log), Err(TraceError::NoTlcTrace)));
    }

    #[test]
    fn tlc_assignment_without_a_state_is_an_error() {
        assert!(matches!(
            parse_tlc("/\\ x = 1\n"),
            Err(TraceError::AssignmentOutsideState { line }) if line == "/\\ x = 1"
        ));
    }

    #[test]
    fn tlc_prefix_is_kept_when_a_name_would_vanish() {
        let trace =
            parse_tlc("State 1: <Initial predicate>\n/\\ a_x = 1\n/\\ a_ = 2\n").expect("parses");
        let names: Vec<&str> = trace.states[0].vars.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["a_x", "a_"]);
    }
}
