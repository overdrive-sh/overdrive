//! JSON state file at `.context/roadmap-sync-state.json`. Records the
//! provisioned Project v2 metadata (from `init`) and maps roadmap ID ->
//! created issue number + URL + timestamp (from `sync`). Used for both
//! `init` idempotency and `sync --resume`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use eyre::{Result, eyre};
use serde::{Deserialize, Serialize};

pub const STATE_FILE: &str = ".context/roadmap-sync-state.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedIssue {
    pub issue_number: u64,
    pub url: String,
    pub created_at: String,
    /// Set once the Pass 2 body-rewrite has resolved dependency `#N` refs.
    #[serde(default)]
    pub body_finalized: bool,
}

/// Recorded Project v2 metadata. Written by `init`; read by `sync` so the
/// operator doesn't have to supply `--project-number` on every run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub owner: String,
    pub number: u64,
    pub title: String,
    /// Project node ID (the `PVT_…` GraphQL ID, required by `item-edit
    /// --project-id`).
    pub project_id: String,
    /// Field ID for the `Phase` single-select field.
    pub phase_field_id: String,
    /// Field ID for the `Depends on` text field.
    pub depends_on_field_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncState {
    /// Provisioned by `roadmap init`. Absent on a fresh workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectMeta>,
    /// Maps roadmap ID (e.g. "1.12") -> created issue.
    #[serde(default)]
    pub created: BTreeMap<String, CreatedIssue>,
}

impl SyncState {
    pub fn path(workspace_root: &Path) -> PathBuf {
        workspace_root.join(STATE_FILE)
    }

    pub fn load(workspace_root: &Path) -> Result<Self> {
        let path = Self::path(workspace_root);
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| eyre!("failed to read {}: {e}", path.display()))?;
        serde_json::from_str(&content).map_err(|e| eyre!("failed to parse {}: {e}", path.display()))
    }

    pub fn save(&self, workspace_root: &Path) -> Result<()> {
        let path = Self::path(workspace_root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| eyre!("failed to create {}: {e}", parent.display()))?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content + "\n")
            .map_err(|e| eyre!("failed to write {}: {e}", path.display()))?;
        Ok(())
    }

    pub fn issue_for(&self, roadmap_id: &str) -> Option<&CreatedIssue> {
        self.created.get(roadmap_id)
    }

    pub fn record(&mut self, roadmap_id: String, issue: CreatedIssue) {
        self.created.insert(roadmap_id, issue);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Empty state — `project` absent, `created` empty — should round-trip
    /// cleanly. The `project` key must be *omitted* (not `null`) so a
    /// state file from an older version without `project` parses and a
    /// fresh-write doesn't pollute the file with nulls.
    #[test]
    fn empty_state_omits_project_key() {
        let state = SyncState::default();
        let json = serde_json::to_string(&state).unwrap();
        assert!(!json.contains("project"), "expected no `project` key, got: {json}");
        let parsed: SyncState = serde_json::from_str(&json).unwrap();
        assert!(parsed.project.is_none());
        assert!(parsed.created.is_empty());
    }

    /// A state file with only the `project` section (the `init`-output
    /// shape before any `sync` has run) must round-trip without losing
    /// fields.
    #[test]
    fn project_only_state_roundtrips() {
        let state = SyncState {
            project: Some(ProjectMeta {
                owner: "overdrive-sh".into(),
                number: 5,
                title: "Overdrive Roadmap".into(),
                project_id: "PVT_example".into(),
                phase_field_id: "PVTSSF_phase".into(),
                depends_on_field_id: "PVTF_depends".into(),
                created_at: "2026-04-21T12:00:00Z".into(),
            }),
            created: BTreeMap::new(),
        };
        let json = serde_json::to_string(&state).unwrap();
        let parsed: SyncState = serde_json::from_str(&json).unwrap();
        let meta = parsed.project.expect("project metadata");
        assert_eq!(meta.number, 5);
        assert_eq!(meta.owner, "overdrive-sh");
        assert_eq!(meta.phase_field_id, "PVTSSF_phase");
        assert!(parsed.created.is_empty());
    }

    /// State files from before the `project` section was added must parse
    /// cleanly as absent project, preserving the existing `created` map.
    #[test]
    fn legacy_state_without_project_parses() {
        let legacy = r#"{
            "created": {
                "1.1": {
                    "issue_number": 10,
                    "url": "https://github.com/o/r/issues/10",
                    "created_at": "2026-04-21T00:00:00Z"
                }
            }
        }"#;
        let parsed: SyncState = serde_json::from_str(legacy).unwrap();
        assert!(parsed.project.is_none());
        assert_eq!(parsed.created.len(), 1);
        assert_eq!(parsed.created["1.1"].issue_number, 10);
    }
}
