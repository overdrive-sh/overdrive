//! Markdown table parsing for `.context/roadmap-issues.md`.
//!
//! The file layout is:
//! - Seven `## Phase N — …` H2 sections.
//! - Each phase has a markdown table with 6 columns: `# | Title | Areas |
//!   Type | Source | Notes`.
//! - Row ID is `phase.index` (e.g. `1.1`, `4.16`).
//! - Areas / Type cells wrap bare names in backticks (e.g. `` `control-plane` ``).
//!   Areas are space-separated; type is one label.
//! - The "## Summary" and "## Open decisions" sections at the bottom are
//!   ignored.

use std::path::Path;

use eyre::{Result, bail, eyre};
use regex::Regex;

/// One parsed roadmap row.
#[derive(Debug, Clone)]
pub struct RoadmapRow {
    /// e.g. "1.12" — phase.index.
    pub id: String,
    pub phase: u8,
    pub title: String,
    /// Bare names without the `area/` prefix.
    pub areas: Vec<String>,
    /// Bare name without the `type/` prefix.
    pub r#type: String,
    /// Whitepaper section refs, e.g. "§4, §12".
    pub source: String,
    pub notes: String,
}

/// Parse the roadmap markdown file into rows.
pub fn parse_file(path: &Path) -> Result<Vec<RoadmapRow>> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| eyre!("failed to read {}: {e}", path.display()))?;
    parse_str(&content)
}

/// Parse roadmap markdown content into rows.
pub fn parse_str(content: &str) -> Result<Vec<RoadmapRow>> {
    let phase_re = Regex::new(r"^##\s+Phase\s+(\d+)\s+—")?;
    // A table row looks like: | 1.12 | Title ... | `a` `b` | `primitive` | §4 | Notes |
    // We match the leading ID as `\d+\.\d+` to skip header and separator rows.
    let row_re = Regex::new(r"^\|\s*(\d+)\.(\d+)\s*\|(.+)\|\s*$")?;

    let mut rows = Vec::new();
    let mut current_phase: Option<u8> = None;
    // Stop parsing when we hit a non-phase H2 section (e.g. "## Summary").
    let mut in_phase_section = false;

    for line in content.lines() {
        if let Some(caps) = phase_re.captures(line) {
            let phase: u8 = caps
                .get(1)
                .ok_or_else(|| eyre!("phase regex matched without group 1"))?
                .as_str()
                .parse()?;
            current_phase = Some(phase);
            in_phase_section = true;
            continue;
        }
        if line.starts_with("## ") && !phase_re.is_match(line) {
            // Left the phase section (e.g. "## Summary").
            in_phase_section = false;
            continue;
        }
        if !in_phase_section {
            continue;
        }
        let Some(phase) = current_phase else { continue };
        let Some(caps) = row_re.captures(line) else { continue };
        let major: u8 =
            caps.get(1).ok_or_else(|| eyre!("row regex group 1 missing"))?.as_str().parse()?;
        let minor: u16 =
            caps.get(2).ok_or_else(|| eyre!("row regex group 2 missing"))?.as_str().parse()?;
        if major != phase {
            bail!("row {major}.{minor} in phase {phase} section (mismatch)");
        }
        let rest = caps.get(3).ok_or_else(|| eyre!("row regex group 3 missing"))?.as_str();
        // Split the remaining cells by unescaped `|`. GitHub-flavored
        // markdown escapes in-cell pipes as `\|` — row 3.10 uses these in
        // its notes cell. We pass through `\|` then un-escape.
        let raw_cells = split_unescaped_pipes(rest);
        if raw_cells.len() < 5 {
            bail!("row {major}.{minor} has {} cells, expected >= 5", raw_cells.len());
        }
        let raw_cells = if raw_cells.last().is_some_and(|c| c.trim().is_empty()) {
            &raw_cells[..raw_cells.len() - 1]
        } else {
            &raw_cells[..]
        };
        if raw_cells.len() != 5 {
            bail!(
                "row {major}.{minor}: expected 5 data cells after trimming trailing empty, got {}",
                raw_cells.len()
            );
        }
        let cells: Vec<String> = raw_cells.iter().map(|c| c.replace("\\|", "|")).collect();

        let title = cells[0].trim().to_string();
        let areas = parse_backticked_labels(&cells[1]);
        let type_labels = parse_backticked_labels(&cells[2]);
        if type_labels.len() != 1 {
            bail!("row {major}.{minor}: expected exactly one type label, got {:?}", type_labels);
        }
        let source = cells[3].trim().to_string();
        let notes = cells[4].trim().to_string();

        rows.push(RoadmapRow {
            id: format!("{major}.{minor}"),
            phase,
            title,
            areas,
            r#type: type_labels.into_iter().next().unwrap_or_default(),
            source,
            notes,
        });
    }

    Ok(rows)
}

/// Split a string on `|`, but treat `\|` as a literal escaped pipe that
/// stays inside the current cell.
fn split_unescaped_pipes(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                if next == '|' {
                    cur.push('\\');
                    cur.push('|');
                    chars.next();
                    continue;
                }
            }
            cur.push(c);
        } else if c == '|' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

/// Extract bare names from a cell like `` `control-plane` `security` ``.
fn parse_backticked_labels(cell: &str) -> Vec<String> {
    // Lazy regex is fine for a one-off script; we only parse 116 rows.
    let re = Regex::new(r"`([^`]+)`").expect("static regex");
    re.captures_iter(cell).filter_map(|c| c.get(1).map(|m| m.as_str().to_string())).collect()
}

/// Extract roadmap ID dependencies from a notes cell. Matches `\d\.\d+` refs;
/// de-duplicates; drops self-references and refs not in `known_ids`.
pub fn extract_dependencies(row: &RoadmapRow, known_ids: &[String]) -> Vec<String> {
    let re = Regex::new(r"\b(\d+\.\d+)\b").expect("static regex");
    let mut deps: Vec<String> = re
        .captures_iter(&row.notes)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .filter(|id| id != &row.id && known_ids.iter().any(|k| k == id))
        .collect();
    deps.sort();
    deps.dedup();
    deps
}

#[cfg(test)]
#[allow(clippy::useless_vec, clippy::needless_raw_string_hashes)]
mod tests {
    use super::*;

    const SAMPLE: &str = r"
## Phase 1 — Single-Node MVP

| # | Title | Areas | Type | Source | Notes |
|---|---|---|---|---|---|
| 1.1 | Define core data model | `control-plane` | `primitive` | §4 | Foo |
| 1.12 | Job-lifecycle reconciler | `control-plane` | `primitive` | §18 | drives 1.8 and 1.7 |

## Phase 2 — Dataplane

| # | Title | Areas | Type | Source | Notes |
|---|---|---|---|---|---|
| 2.13 | IdentityMgr | `security` `control-plane` | `primitive` | §8 | Bar |

## Summary

| Phase | Issues |
|---|---|
| 1 | 13 |
";

    #[test]
    fn parses_rows_and_skips_summary() {
        let rows = parse_str(SAMPLE).expect("parse");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].id, "1.1");
        assert_eq!(rows[1].id, "1.12");
        assert_eq!(rows[2].id, "2.13");
        assert_eq!(rows[2].areas, vec!["security", "control-plane"]);
        assert_eq!(rows[2].r#type, "primitive");
    }

    #[test]
    fn extracts_dependencies() {
        let rows = parse_str(SAMPLE).expect("parse");
        let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
        let deps = extract_dependencies(&rows[1], &ids);
        // 1.8 and 1.7 are not known IDs (not in the sample), so they filter
        // out. Only references to known rows are retained.
        assert!(deps.is_empty());
    }

    #[test]
    fn extracts_dependencies_when_known() {
        let rows = vec![
            RoadmapRow {
                id: "4.16".into(),
                phase: 4,
                title: "Multi-stage".into(),
                areas: vec![],
                r#type: "primitive".into(),
                source: String::new(),
                notes: "built on top of 4.14 + 4.15 reconcilers".into(),
            },
            RoadmapRow {
                id: "4.14".into(),
                phase: 4,
                title: "Rolling deploy".into(),
                areas: vec![],
                r#type: "primitive".into(),
                source: String::new(),
                notes: String::new(),
            },
            RoadmapRow {
                id: "4.15".into(),
                phase: 4,
                title: "Canary".into(),
                areas: vec![],
                r#type: "primitive".into(),
                source: String::new(),
                notes: String::new(),
            },
        ];
        let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
        let deps = extract_dependencies(&rows[0], &ids);
        assert_eq!(deps, vec!["4.14", "4.15"]);
    }
}
