//! `yaml-free-cli` — CI gate enforcing ADR-0019 §Consequences →
//! Enforcement on the `overdrive-cli` dependency graph.
//!
//! ADR-0019 supersedes ADR-0010 §R2 and mandates TOML as the on-disk
//! format for `~/.overdrive/config`. The `serde_yaml` backend is
//! archived upstream; its community fork `serde_yml` is governance-
//! uncertain. Neither belongs in `overdrive-cli`'s dependency graph.
//!
//! This gate walks the full resolved dependency graph (with deps, i.e.
//! transitive) for the `overdrive-cli` package. If `serde_yaml` or
//! `serde_yml` appears — directly or transitively — the gate fails
//! with a structured error pointing at the offending chain.
//!
//! # Relationship to `dst-lint`
//!
//! `dst-lint` (see [`crate::dst_lint`]) is a *source-code* lint over
//! banned APIs in `crate_class = "core"` crates. This gate is a
//! *dependency-graph* lint over a specific binary crate
//! (`overdrive-cli`). The two are orthogonal: dst-lint catches
//! `Instant::now()` at the call site; yaml-free-cli catches a
//! transitive `serde_yaml` pull-in before it reaches code.
//!
//! # Determinism boundary
//!
//! Like [`crate::dst_lint`], this module runs inside `xtask` (a
//! binary crate, `crate_class = "binary"`). The pure entry point
//! [`scan_metadata`] takes already-materialised
//! `cargo_metadata::Metadata` and returns a list of
//! [`ForbiddenDependency`] records — trivially testable against
//! synthetic inputs without shelling out to `cargo metadata`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use cargo_metadata::{DependencyKind, Metadata, PackageId};
use color_eyre::eyre::{Context, Result, bail};

/// The crate whose dependency graph must stay YAML-free.
///
/// ADR-0019 scopes this to `overdrive-cli` specifically — the
/// control-plane crate may still reasonably depend on TOML/YAML for
/// unrelated reasons (though at time of writing it does not).
pub const TARGET_CRATE: &str = "overdrive-cli";

/// Forbidden package names. Matched case-sensitively against
/// `cargo_metadata::Package::name`.
pub const FORBIDDEN_CRATES: &[&str] = &["serde_yaml", "serde_yml"];

/// A single forbidden dependency discovered in the resolved graph of
/// [`TARGET_CRATE`], along with the chain of packages that pulled it
/// in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForbiddenDependency {
    /// Name of the forbidden package (e.g. `serde_yaml`).
    pub name: String,
    /// Chain from [`TARGET_CRATE`] to the forbidden package, in
    /// reachability order. First element is `TARGET_CRATE`; last is
    /// `name`.
    pub chain: Vec<String>,
}

/// Scan resolved workspace metadata for forbidden YAML dependencies.
///
/// Given metadata from `cargo metadata --deps`, returns every
/// forbidden dependency reachable from [`TARGET_CRATE`]. `Err` on
/// structural problems (target crate not found, resolve missing).
pub fn scan_metadata(metadata: &Metadata) -> Result<Vec<ForbiddenDependency>> {
    let resolve = metadata
        .resolve
        .as_ref()
        .ok_or_else(|| color_eyre::eyre::eyre!("cargo metadata returned no resolve graph"))?;

    // Locate the target crate's PackageId.
    let target_id: &PackageId = metadata
        .packages
        .iter()
        .find(|p| p.name == TARGET_CRATE)
        .map(|p| &p.id)
        .ok_or_else(|| {
            color_eyre::eyre::eyre!("target crate `{TARGET_CRATE}` not found in workspace metadata")
        })?;

    // Build a name-lookup and a non-dev dep adjacency list. Dev-deps
    // are excluded — they do not ship with a binary and the
    // "absence in the overdrive-cli graph" claim is about runtime
    // reachability, not test tooling. An integration test that
    // round-trips YAML as a negative fixture should not fail this gate.
    let mut name_of: HashMap<PackageId, String> = HashMap::new();
    for pkg in &metadata.packages {
        name_of.insert(pkg.id.clone(), pkg.name.clone());
    }

    let mut edges: HashMap<PackageId, Vec<PackageId>> = HashMap::new();
    for node in &resolve.nodes {
        let non_dev: Vec<PackageId> = node
            .deps
            .iter()
            .filter(|d| {
                // A dep kind of `Normal` or `Build` ships with the
                // binary; `Development` does not. `dep_kinds` carries
                // one entry per (dep, kind, target); keep the dep if
                // any non-dev kind is present.
                d.dep_kinds.iter().any(|k| k.kind != DependencyKind::Development)
            })
            .map(|d| d.pkg.clone())
            .collect();
        edges.insert(node.id.clone(), non_dev);
    }

    // BFS from the target, tracking the parent so we can reconstruct
    // the chain when a forbidden node is hit. `parent[n] = Some(p)`
    // means `p` pulled in `n`; the root has `None`.
    let mut parent: HashMap<PackageId, Option<PackageId>> = HashMap::new();
    let mut seen: HashSet<PackageId> = HashSet::new();
    let mut queue: VecDeque<PackageId> = VecDeque::new();
    parent.insert(target_id.clone(), None);
    seen.insert(target_id.clone());
    queue.push_back(target_id.clone());

    let mut hits: Vec<ForbiddenDependency> = Vec::new();
    while let Some(id) = queue.pop_front() {
        if let Some(name) = name_of.get(&id) {
            if FORBIDDEN_CRATES.contains(&name.as_str()) && &id != target_id {
                hits.push(ForbiddenDependency {
                    name: name.clone(),
                    chain: reconstruct_chain(&id, &parent, &name_of),
                });
                // Don't recurse past a forbidden node — one chain per
                // forbidden package is enough to motivate the failure.
                continue;
            }
        }
        if let Some(children) = edges.get(&id) {
            for child in children {
                if seen.insert(child.clone()) {
                    parent.insert(child.clone(), Some(id.clone()));
                    queue.push_back(child.clone());
                }
            }
        }
    }

    Ok(hits)
}

/// Walk `parent` backwards from `id` to the root, mapping `PackageId`s
/// to names. Returns the chain in root→id order.
fn reconstruct_chain(
    id: &PackageId,
    parent: &HashMap<PackageId, Option<PackageId>>,
    name_of: &HashMap<PackageId, String>,
) -> Vec<String> {
    let mut chain: Vec<String> = Vec::new();
    let mut cursor: Option<&PackageId> = Some(id);
    while let Some(current) = cursor {
        let name = name_of.get(current).cloned().unwrap_or_else(|| "<unknown>".to_string());
        chain.push(name);
        cursor = parent.get(current).and_then(Option::as_ref);
    }
    chain.reverse();
    chain
}

/// Render a single forbidden-dependency hit as a rustc-style stderr
/// block.
pub fn render_violation(v: &ForbiddenDependency) -> String {
    let arrow_chain = v.chain.join(" → ");
    format!(
        "error: forbidden dependency in `{target}` graph: `{name}`\n  \
         --> via {arrow_chain}\n  \
         |\n  \
         = help: ADR-0019 supersedes ADR-0010 §R2. `~/.overdrive/config` is TOML;\n  \
                 `serde_yaml` / `serde_yml` must not appear in `{target}`'s\n  \
                 resolved dependency graph. Remove the dependency or replace\n  \
                 with the `toml` crate.\n  \
         = note: see docs/product/architecture/adr-0019-operator-config-format-toml.md\n",
        target = TARGET_CRATE,
        name = v.name,
    )
}

/// CLI entry point for the `yaml-free-cli` xtask subcommand.
///
/// Runs `cargo metadata --manifest-path <mp>` with deps, scans the
/// result, writes violations to stderr, and returns `Err` iff any
/// were found. `Ok(())` means the `overdrive-cli` graph is YAML-free.
pub fn run(manifest_path: &Path) -> Result<()> {
    let metadata = cargo_metadata::MetadataCommand::new()
        .manifest_path(manifest_path)
        .exec()
        .with_context(|| format!("cargo metadata for {}", manifest_path.display()))?;

    let hits = scan_metadata(&metadata)?;
    if hits.is_empty() {
        eprintln!(
            "yaml-free-cli: `{TARGET_CRATE}` dependency graph is clean \
             (no {FORBIDDEN_CRATES:?})"
        );
        return Ok(());
    }

    for hit in &hits {
        eprint!("{}", render_violation(hit));
    }
    eprintln!("yaml-free-cli: {} forbidden dependency chain(s) in `{TARGET_CRATE}`", hits.len());
    bail!("forbidden YAML dependency in `{TARGET_CRATE}` graph (ADR-0019)");
}

// ---------------------------------------------------------------------------
// Unit tests — graph-walker logic against synthetic metadata
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal `cargo_metadata::Metadata` from the JSON that
    /// `cargo metadata --format-version 1 --manifest-path …` would
    /// produce. Using real JSON (rather than constructing the strongly-
    /// typed structs by hand) keeps the test self-documenting and
    /// guards against `cargo_metadata` field drift — if the crate ever
    /// changes shape, the test fails to parse rather than silently
    /// diverging from production.
    fn metadata_from_json(json: &str) -> Metadata {
        serde_json::from_str(json).expect("parse fixture metadata")
    }

    /// A workspace containing just `overdrive-cli` and its (clean)
    /// direct deps must produce zero hits.
    #[test]
    fn clean_graph_yields_no_violations() {
        let json = r#"{
            "packages": [
                {
                    "name": "overdrive-cli", "version": "0.1.0",
                    "id": "path+file:///w/overdrive-cli#overdrive-cli@0.1.0",
                    "license": null, "license_file": null, "description": null,
                    "source": null,
                    "dependencies": [],
                    "targets": [], "features": {},
                    "manifest_path": "/w/overdrive-cli/Cargo.toml",
                    "metadata": null, "publish": null,
                    "authors": [], "categories": [], "keywords": [],
                    "readme": null, "repository": null, "homepage": null,
                    "documentation": null, "edition": "2021",
                    "links": null, "default_run": null, "rust_version": null
                },
                {
                    "name": "toml", "version": "0.8.0",
                    "id": "registry+https://github.com/rust-lang/crates.io-index#toml@0.8.0",
                    "license": null, "license_file": null, "description": null,
                    "source": "registry+https://github.com/rust-lang/crates.io-index",
                    "dependencies": [],
                    "targets": [], "features": {},
                    "manifest_path": "/c/toml/Cargo.toml",
                    "metadata": null, "publish": null,
                    "authors": [], "categories": [], "keywords": [],
                    "readme": null, "repository": null, "homepage": null,
                    "documentation": null, "edition": "2021",
                    "links": null, "default_run": null, "rust_version": null
                }
            ],
            "workspace_members": ["path+file:///w/overdrive-cli#overdrive-cli@0.1.0"],
            "workspace_default_members": ["path+file:///w/overdrive-cli#overdrive-cli@0.1.0"],
            "resolve": {
                "nodes": [
                    {
                        "id": "path+file:///w/overdrive-cli#overdrive-cli@0.1.0",
                        "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#toml@0.8.0"],
                        "deps": [
                            {
                                "name": "toml",
                                "pkg": "registry+https://github.com/rust-lang/crates.io-index#toml@0.8.0",
                                "dep_kinds": [{"kind": null, "target": null, "extern_name": null}]
                            }
                        ],
                        "features": []
                    },
                    {
                        "id": "registry+https://github.com/rust-lang/crates.io-index#toml@0.8.0",
                        "dependencies": [], "deps": [], "features": []
                    }
                ],
                "root": "path+file:///w/overdrive-cli#overdrive-cli@0.1.0"
            },
            "target_directory": "/w/target",
            "version": 1,
            "workspace_root": "/w",
            "metadata": null
        }"#;
        let metadata = metadata_from_json(json);
        let hits = scan_metadata(&metadata).expect("scan");
        assert!(hits.is_empty(), "expected no violations, got {hits:?}");
    }

    /// A direct `serde_yaml` dependency must be detected, and the
    /// chain must show `overdrive-cli → serde_yaml`.
    #[test]
    fn direct_serde_yaml_dependency_is_detected() {
        let json = r#"{
            "packages": [
                {
                    "name": "overdrive-cli", "version": "0.1.0",
                    "id": "path+file:///w/overdrive-cli#overdrive-cli@0.1.0",
                    "license": null, "license_file": null, "description": null,
                    "source": null,
                    "dependencies": [], "targets": [], "features": {},
                    "manifest_path": "/w/overdrive-cli/Cargo.toml",
                    "metadata": null, "publish": null,
                    "authors": [], "categories": [], "keywords": [],
                    "readme": null, "repository": null, "homepage": null,
                    "documentation": null, "edition": "2021",
                    "links": null, "default_run": null, "rust_version": null
                },
                {
                    "name": "serde_yaml", "version": "0.9.0",
                    "id": "registry+https://github.com/rust-lang/crates.io-index#serde_yaml@0.9.0",
                    "license": null, "license_file": null, "description": null,
                    "source": "registry+https://github.com/rust-lang/crates.io-index",
                    "dependencies": [], "targets": [], "features": {},
                    "manifest_path": "/c/serde_yaml/Cargo.toml",
                    "metadata": null, "publish": null,
                    "authors": [], "categories": [], "keywords": [],
                    "readme": null, "repository": null, "homepage": null,
                    "documentation": null, "edition": "2021",
                    "links": null, "default_run": null, "rust_version": null
                }
            ],
            "workspace_members": ["path+file:///w/overdrive-cli#overdrive-cli@0.1.0"],
            "workspace_default_members": ["path+file:///w/overdrive-cli#overdrive-cli@0.1.0"],
            "resolve": {
                "nodes": [
                    {
                        "id": "path+file:///w/overdrive-cli#overdrive-cli@0.1.0",
                        "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#serde_yaml@0.9.0"],
                        "deps": [
                            {
                                "name": "serde_yaml",
                                "pkg": "registry+https://github.com/rust-lang/crates.io-index#serde_yaml@0.9.0",
                                "dep_kinds": [{"kind": null, "target": null, "extern_name": null}]
                            }
                        ],
                        "features": []
                    },
                    {
                        "id": "registry+https://github.com/rust-lang/crates.io-index#serde_yaml@0.9.0",
                        "dependencies": [], "deps": [], "features": []
                    }
                ],
                "root": "path+file:///w/overdrive-cli#overdrive-cli@0.1.0"
            },
            "target_directory": "/w/target",
            "version": 1,
            "workspace_root": "/w",
            "metadata": null
        }"#;
        let metadata = metadata_from_json(json);
        let hits = scan_metadata(&metadata).expect("scan");
        assert_eq!(hits.len(), 1, "expected exactly one violation, got {hits:?}");
        assert_eq!(hits[0].name, "serde_yaml");
        assert_eq!(hits[0].chain, vec!["overdrive-cli".to_string(), "serde_yaml".to_string()]);
    }

    /// A transitive `serde_yml` dependency through an intermediate
    /// must be detected with the full chain reported.
    #[test]
    fn transitive_serde_yml_dependency_is_detected_with_chain() {
        let json = r#"{
            "packages": [
                {
                    "name": "overdrive-cli", "version": "0.1.0",
                    "id": "path+file:///w/overdrive-cli#overdrive-cli@0.1.0",
                    "license": null, "license_file": null, "description": null,
                    "source": null,
                    "dependencies": [], "targets": [], "features": {},
                    "manifest_path": "/w/overdrive-cli/Cargo.toml",
                    "metadata": null, "publish": null,
                    "authors": [], "categories": [], "keywords": [],
                    "readme": null, "repository": null, "homepage": null,
                    "documentation": null, "edition": "2021",
                    "links": null, "default_run": null, "rust_version": null
                },
                {
                    "name": "intermediate", "version": "1.0.0",
                    "id": "registry+https://github.com/rust-lang/crates.io-index#intermediate@1.0.0",
                    "license": null, "license_file": null, "description": null,
                    "source": "registry+https://github.com/rust-lang/crates.io-index",
                    "dependencies": [], "targets": [], "features": {},
                    "manifest_path": "/c/intermediate/Cargo.toml",
                    "metadata": null, "publish": null,
                    "authors": [], "categories": [], "keywords": [],
                    "readme": null, "repository": null, "homepage": null,
                    "documentation": null, "edition": "2021",
                    "links": null, "default_run": null, "rust_version": null
                },
                {
                    "name": "serde_yml", "version": "0.0.12",
                    "id": "registry+https://github.com/rust-lang/crates.io-index#serde_yml@0.0.12",
                    "license": null, "license_file": null, "description": null,
                    "source": "registry+https://github.com/rust-lang/crates.io-index",
                    "dependencies": [], "targets": [], "features": {},
                    "manifest_path": "/c/serde_yml/Cargo.toml",
                    "metadata": null, "publish": null,
                    "authors": [], "categories": [], "keywords": [],
                    "readme": null, "repository": null, "homepage": null,
                    "documentation": null, "edition": "2021",
                    "links": null, "default_run": null, "rust_version": null
                }
            ],
            "workspace_members": ["path+file:///w/overdrive-cli#overdrive-cli@0.1.0"],
            "workspace_default_members": ["path+file:///w/overdrive-cli#overdrive-cli@0.1.0"],
            "resolve": {
                "nodes": [
                    {
                        "id": "path+file:///w/overdrive-cli#overdrive-cli@0.1.0",
                        "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#intermediate@1.0.0"],
                        "deps": [
                            {
                                "name": "intermediate",
                                "pkg": "registry+https://github.com/rust-lang/crates.io-index#intermediate@1.0.0",
                                "dep_kinds": [{"kind": null, "target": null, "extern_name": null}]
                            }
                        ],
                        "features": []
                    },
                    {
                        "id": "registry+https://github.com/rust-lang/crates.io-index#intermediate@1.0.0",
                        "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#serde_yml@0.0.12"],
                        "deps": [
                            {
                                "name": "serde_yml",
                                "pkg": "registry+https://github.com/rust-lang/crates.io-index#serde_yml@0.0.12",
                                "dep_kinds": [{"kind": null, "target": null, "extern_name": null}]
                            }
                        ],
                        "features": []
                    },
                    {
                        "id": "registry+https://github.com/rust-lang/crates.io-index#serde_yml@0.0.12",
                        "dependencies": [], "deps": [], "features": []
                    }
                ],
                "root": "path+file:///w/overdrive-cli#overdrive-cli@0.1.0"
            },
            "target_directory": "/w/target",
            "version": 1,
            "workspace_root": "/w",
            "metadata": null
        }"#;
        let metadata = metadata_from_json(json);
        let hits = scan_metadata(&metadata).expect("scan");
        assert_eq!(hits.len(), 1, "expected exactly one violation, got {hits:?}");
        assert_eq!(hits[0].name, "serde_yml");
        assert_eq!(
            hits[0].chain,
            vec!["overdrive-cli".to_string(), "intermediate".to_string(), "serde_yml".to_string(),]
        );
    }

    /// Dev-dependencies must NOT trip the gate. A crate that pulls in
    /// `serde_yaml` as a `[dev-dependencies]` entry is not shipping
    /// YAML in the binary — test-only tooling is out of scope.
    #[test]
    fn dev_only_dependency_is_not_a_violation() {
        let json = r#"{
            "packages": [
                {
                    "name": "overdrive-cli", "version": "0.1.0",
                    "id": "path+file:///w/overdrive-cli#overdrive-cli@0.1.0",
                    "license": null, "license_file": null, "description": null,
                    "source": null,
                    "dependencies": [], "targets": [], "features": {},
                    "manifest_path": "/w/overdrive-cli/Cargo.toml",
                    "metadata": null, "publish": null,
                    "authors": [], "categories": [], "keywords": [],
                    "readme": null, "repository": null, "homepage": null,
                    "documentation": null, "edition": "2021",
                    "links": null, "default_run": null, "rust_version": null
                },
                {
                    "name": "serde_yaml", "version": "0.9.0",
                    "id": "registry+https://github.com/rust-lang/crates.io-index#serde_yaml@0.9.0",
                    "license": null, "license_file": null, "description": null,
                    "source": "registry+https://github.com/rust-lang/crates.io-index",
                    "dependencies": [], "targets": [], "features": {},
                    "manifest_path": "/c/serde_yaml/Cargo.toml",
                    "metadata": null, "publish": null,
                    "authors": [], "categories": [], "keywords": [],
                    "readme": null, "repository": null, "homepage": null,
                    "documentation": null, "edition": "2021",
                    "links": null, "default_run": null, "rust_version": null
                }
            ],
            "workspace_members": ["path+file:///w/overdrive-cli#overdrive-cli@0.1.0"],
            "workspace_default_members": ["path+file:///w/overdrive-cli#overdrive-cli@0.1.0"],
            "resolve": {
                "nodes": [
                    {
                        "id": "path+file:///w/overdrive-cli#overdrive-cli@0.1.0",
                        "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#serde_yaml@0.9.0"],
                        "deps": [
                            {
                                "name": "serde_yaml",
                                "pkg": "registry+https://github.com/rust-lang/crates.io-index#serde_yaml@0.9.0",
                                "dep_kinds": [{"kind": "dev", "target": null, "extern_name": null}]
                            }
                        ],
                        "features": []
                    },
                    {
                        "id": "registry+https://github.com/rust-lang/crates.io-index#serde_yaml@0.9.0",
                        "dependencies": [], "deps": [], "features": []
                    }
                ],
                "root": "path+file:///w/overdrive-cli#overdrive-cli@0.1.0"
            },
            "target_directory": "/w/target",
            "version": 1,
            "workspace_root": "/w",
            "metadata": null
        }"#;
        let metadata = metadata_from_json(json);
        let hits = scan_metadata(&metadata).expect("scan");
        assert!(hits.is_empty(), "dev-only dependencies must not be flagged (found {hits:?})");
    }

    /// Missing target crate → error, not silent pass.
    #[test]
    fn missing_target_crate_is_error() {
        let json = r#"{
            "packages": [],
            "workspace_members": [],
            "workspace_default_members": [],
            "resolve": {"nodes": [], "root": null},
            "target_directory": "/w/target",
            "version": 1,
            "workspace_root": "/w",
            "metadata": null
        }"#;
        let metadata = metadata_from_json(json);
        let err = scan_metadata(&metadata).expect_err("must error");
        assert!(
            err.to_string().contains(TARGET_CRATE),
            "error must name the missing target crate; got: {err}"
        );
    }
}
