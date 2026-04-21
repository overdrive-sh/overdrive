//! Thin shell-out wrappers around `gh`. We rely on the user's existing
//! `gh auth` state; this is a one-off utility, not a production library.

use std::process::{Command, Output, Stdio};

use eyre::{Result, bail, eyre};
use serde::Deserialize;

/// Verify `gh` is installed and authenticated. Returns a clear error message
/// if not.
pub fn check_auth() -> Result<()> {
    which("gh").map_err(|_| {
        eyre!(
            "`gh` not found on PATH. Install it with: brew install gh (or see https://cli.github.com/)"
        )
    })?;
    let out = Command::new("gh").args(["auth", "status"]).output()?;
    if !out.status.success() {
        bail!(
            "`gh auth status` failed — run `gh auth login` first.\nstderr: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn which(binary: &str) -> Result<()> {
    let found = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {binary}"))
        .stdout(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !found {
        bail!("`{binary}` not found on PATH");
    }
    Ok(())
}

/// True if a label with exactly `name` exists in `repo`.
pub fn label_exists(repo: &str, name: &str) -> Result<bool> {
    // `gh label list --json name` returns all labels; filter in Rust to
    // avoid substring-match false positives from `--search`.
    let out = Command::new("gh")
        .args(["label", "list", "--repo", repo, "--json", "name", "--limit", "1000"])
        .output()?;
    if !out.status.success() {
        bail!("gh label list failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    #[derive(Deserialize)]
    struct Label {
        name: String,
    }
    let labels: Vec<Label> = serde_json::from_slice(&out.stdout)?;
    Ok(labels.iter().any(|l| l.name == name))
}

/// Create a label. No-op if it already exists (caller checks).
pub fn create_label(repo: &str, name: &str, color: &str, description: &str) -> Result<()> {
    let out = Command::new("gh")
        .args([
            "label",
            "create",
            name,
            "--repo",
            repo,
            "--color",
            color,
            "--description",
            description,
        ])
        .output()?;
    if !out.status.success() {
        bail!("gh label create {name} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Minimal Project v2 info we need to verify it exists and to look up custom
/// field IDs.
#[derive(Debug, Deserialize)]
pub struct ProjectFieldList {
    pub fields: ProjectFieldListInner,
}

#[derive(Debug, Deserialize)]
pub struct ProjectFieldListInner {
    pub nodes: Vec<ProjectField>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ProjectField {
    SingleSelect { id: String, name: String, options: Vec<ProjectFieldOption> },
    Other { id: String, name: String },
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProjectFieldOption {
    pub id: String,
    pub name: String,
}

impl ProjectField {
    pub fn name(&self) -> &str {
        match self {
            Self::SingleSelect { name, .. } | Self::Other { name, .. } => name,
        }
    }
}

/// Fetch project fields via `gh project field-list`. Validates the project
/// exists under `owner` at `number`.
pub fn project_field_list(owner: &str, number: u64) -> Result<ProjectFieldList> {
    let out = Command::new("gh")
        .args([
            "project",
            "field-list",
            &number.to_string(),
            "--owner",
            owner,
            "--format",
            "json",
            "--limit",
            "100",
        ])
        .output()?;
    if !out.status.success() {
        bail!(
            "gh project field-list failed (is project #{number} under {owner}?): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let parsed: ProjectFieldList = serde_json::from_slice(&out.stdout)
        .map_err(|e| eyre!("failed to parse gh project field-list output: {e}"))?;
    Ok(parsed)
}

/// Look up `gh project view <number> --owner <owner> --format json` to get
/// the project's node ID (required for `item-add`).
pub fn project_node_id(owner: &str, number: u64) -> Result<String> {
    let out = Command::new("gh")
        .args(["project", "view", &number.to_string(), "--owner", owner, "--format", "json"])
        .output()?;
    if !out.status.success() {
        bail!("gh project view failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    #[derive(Deserialize)]
    struct Project {
        id: String,
    }
    let p: Project = serde_json::from_slice(&out.stdout)?;
    Ok(p.id)
}

/// Create an issue, return (issue_number, url).
pub fn create_issue(
    repo: &str,
    title: &str,
    body: &str,
    labels: &[String],
) -> Result<(u64, String)> {
    let mut cmd = Command::new("gh");
    cmd.args(["issue", "create", "--repo", repo, "--title", title, "--body", body]);
    for label in labels {
        cmd.args(["--label", label]);
    }
    let out = cmd.output()?;
    if !out.status.success() {
        bail!("gh issue create failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    // `gh issue create` prints the issue URL on stdout.
    let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let number = parse_issue_number(&url)
        .ok_or_else(|| eyre!("could not parse issue number from gh output: {url}"))?;
    Ok((number, url))
}

/// Parse "https://github.com/owner/repo/issues/42" -> 42.
pub fn parse_issue_number(url: &str) -> Option<u64> {
    url.rsplit('/').next().and_then(|n| n.trim().parse().ok())
}

/// Overwrite an issue body.
pub fn edit_issue_body(repo: &str, issue_number: u64, body: &str) -> Result<()> {
    let out = Command::new("gh")
        .args(["issue", "edit", &issue_number.to_string(), "--repo", repo, "--body", body])
        .output()?;
    if !out.status.success() {
        bail!("gh issue edit failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Add an issue to a Project v2. Returns the item ID (needed for field-set).
pub fn project_item_add(owner: &str, number: u64, issue_url: &str) -> Result<String> {
    let out = Command::new("gh")
        .args([
            "project",
            "item-add",
            &number.to_string(),
            "--owner",
            owner,
            "--url",
            issue_url,
            "--format",
            "json",
        ])
        .output()?;
    if !out.status.success() {
        bail!("gh project item-add failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    #[derive(Deserialize)]
    struct Item {
        id: String,
    }
    let item: Item = serde_json::from_slice(&out.stdout)?;
    Ok(item.id)
}

/// Set a single-select field on a Project v2 item.
pub fn project_field_set_single_select(
    owner: &str,
    project_number: u64,
    project_id: &str,
    item_id: &str,
    field_id: &str,
    option_id: &str,
) -> Result<()> {
    let _ = owner;
    let out = Command::new("gh")
        .args([
            "project",
            "item-edit",
            "--id",
            item_id,
            "--field-id",
            field_id,
            "--project-id",
            project_id,
            "--single-select-option-id",
            option_id,
        ])
        .output()?;
    if !out.status.success() {
        bail!(
            "gh project item-edit (set phase on project #{project_number}) failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Set a free-form text field on a Project v2 item.
pub fn project_field_set_text(
    project_id: &str,
    item_id: &str,
    field_id: &str,
    text: &str,
) -> Result<()> {
    let out = Command::new("gh")
        .args([
            "project",
            "item-edit",
            "--id",
            item_id,
            "--field-id",
            field_id,
            "--project-id",
            project_id,
            "--text",
            text,
        ])
        .output()?;
    if !out.status.success() {
        bail!(
            "gh project item-edit (text field) failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

// Unused-output helper kept for clarity at call sites.
#[allow(dead_code)]
pub fn check_output(label: &str, out: &Output) -> Result<()> {
    if !out.status.success() {
        bail!("{label} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_issue_number_from_url() {
        assert_eq!(
            parse_issue_number("https://github.com/overdrive-sh/overdrive/issues/42"),
            Some(42)
        );
        assert_eq!(parse_issue_number("not a url"), None);
    }
}
