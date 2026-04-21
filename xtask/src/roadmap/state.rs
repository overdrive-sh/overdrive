//! JSON state file at `.context/roadmap-sync-state.json`. Maps roadmap ID
//! -> created issue number + URL + timestamp. Used for `--resume`.

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncState {
    /// Maps roadmap ID (e.g. "1.12") -> created issue.
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
