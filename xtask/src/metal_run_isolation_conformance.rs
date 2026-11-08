//! P2/P5 sole-model conformance through the actual durable owner/control pipe.
//! The driver plays the already-declared controller, not its decision logic.
//! Full-bootstrap producer tests separately prove source-mismatch and missing-rsync
//! decisions. Real systemd owns unit stop; the external ExecStopPost process
//! fixture delays its response before execing the captured production finalizer.
//! State reads actual metadata/phase/terminal/completion, /proc fdinfo, source
//! bytes and systemctl. Model ghosts are excluded. A collector return is compared
//! once with that action's model result, not stored as model-history state.
#![allow(
    clippy::expect_used,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::unnecessary_wraps,
    clippy::needless_pass_by_value,
    clippy::similar_names,
    reason = "test-local real-owner replay and exact Contract Shape declarations"
)]
use itf::Value;
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
#[allow(dead_code, reason = "shared test support also serves the separate acceptance binary")]
#[path = "../tests/acceptance/metal_run_isolation/support.rs"]
mod support;
use support::{Fixture, executable, fields, git, snapshot, success, text, workspace_file};

fn failed(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}
fn poll(mut predicate: impl FnMut() -> bool, label: &str) -> Result {
    let deadline = Instant::now() + Duration::from_secs(7);
    while !predicate() {
        if Instant::now() >= deadline {
            return Err(failed(format!("actual owner never reached {label}")).into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}
fn validated(path: &Path, keys: &[&str], format: Option<&str>) -> Result<BTreeMap<String, String>> {
    let bytes = text(path);
    let actual: Vec<_> =
        bytes.lines().map(|line| line.split_once('=').map_or("", |(key, _)| key)).collect();
    if actual != keys || !bytes.ends_with('\n') {
        return Err(failed(format!("invalid ordered owner record {}", path.display())).into());
    }
    let record = fields(path);
    if let Some(format) = format
        && record.get("format").map(String::as_str) != Some(format)
    {
        return Err(failed("record filename/format mismatch").into());
    }
    Ok(record)
}
fn unit_facts(id: &str) -> Result<BTreeMap<String, String>> {
    let output = Command::new("/usr/bin/systemctl")
        .args([
            "show",
            &format!("overdrive-metal-run-{id}.service"),
            "--property=MainPID,ActiveState,ControlGroup,LoadState",
        ])
        .output()?;
    // A collected transient unit is an actual absent owner, not a fabricated result.
    let values: BTreeMap<String, String> = String::from_utf8(output.stdout)?
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    if ["MainPID", "ActiveState", "ControlGroup", "LoadState"]
        .iter()
        .any(|key| !values.contains_key(*key))
        || (!output.status.success()
            && values.get("LoadState").map(String::as_str) != Some("not-found"))
    {
        return Err(failed(format!(
            "real unit observation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
        .into());
    }
    Ok(values)
}
fn live(facts: &BTreeMap<String, String>) -> bool {
    facts
        .get("ActiveState")
        .is_some_and(|v| matches!(v.as_str(), "active" | "activating" | "deactivating"))
}
fn lock_mode(pid: u32, target: &Path) -> Result<Option<String>> {
    if pid == 0 {
        return Ok(None);
    }
    let directory = PathBuf::from(format!("/proc/{pid}/fd"));
    let Ok(entries) = fs::read_dir(&directory) else {
        return Ok(None);
    };
    for entry in entries {
        let entry = entry?;
        if fs::read_link(entry.path()).ok().as_deref() == Some(target) {
            let Ok(info) = fs::read_to_string(format!(
                "/proc/{pid}/fdinfo/{}",
                entry.file_name().to_string_lossy()
            )) else {
                continue;
            };
            if info.contains("FLOCK") {
                if info.contains("WRITE") {
                    return Ok(Some("exclusive".into()));
                }
                if info.contains("READ") {
                    return Ok(Some("shared".into()));
                }
            }
        }
    }
    Ok(None)
}
struct HeldLock(Child);
impl HeldLock {
    fn try_exclusive(path: &Path, acknowledgement: &Path) -> Result<Option<Self>> {
        let script = format!("printf held > '{}'; cat >/dev/null", acknowledgement.display());
        let mut child = Command::new("/usr/bin/flock")
            .args(["--exclusive", "--nonblock"])
            .arg(path)
            .args(["bash", "-c", &script])
            .stdin(Stdio::piped())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if acknowledgement.is_file() {
                return Ok(Some(Self(child)));
            }
            if let Some(status) = child.try_wait()? {
                if status.code() == Some(1) {
                    return Ok(None);
                }
                return Err(failed("blocking fixture lock failed unexpectedly").into());
            }
            if Instant::now() >= deadline {
                return Err(failed("blocking fixture lock did not resolve").into());
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
    fn shared(path: &Path, acknowledgement: &Path) -> Result<Self> {
        let script = format!("printf held > '{}'; cat >/dev/null", acknowledgement.display());
        let child = Command::new("/usr/bin/flock")
            .arg("--shared")
            .arg(path)
            .args(["bash", "-c", &script])
            .stdin(Stdio::piped())
            .spawn()?;
        poll(|| acknowledgement.is_file(), "actual shared blocker")?;
        Ok(Self(child))
    }
}
impl Drop for HeldLock {
    fn drop(&mut self) {
        drop(self.0.stdin.take());
        let _ = self.0.wait();
    }
}
struct NativeUnit {
    id: String,
    actor: PathBuf,
    directory: PathBuf,
    source: PathBuf,
    tree: PathBuf,
    child: Child,
    expected: BTreeMap<String, String>,
}
impl NativeUnit {
    fn spawn(
        fixture: &Fixture,
        source: &Path,
        tree: &Path,
        expected: &BTreeMap<String, String>,
        label: &str,
        class: &str,
    ) -> Result<Self> {
        let actor = fixture.temp.path().join(label);
        fs::create_dir_all(&actor)?;
        let id =
            format!("{:x}", Sha256::digest(actor.as_os_str().as_encoded_bytes()))[..24].to_owned();
        let captured = actor.join("captured-holder.sh");
        fs::copy(workspace_file("infra/metal/lease-holder.sh"), &captured)?;
        let gate = actor.join("finalizer");
        executable(
            &gate,
            &format!(
                "#!/usr/bin/env bash\nset -eu\nlimit=$((SECONDS+20))\nwhile [ ! -f '{}' ] && kill -0 {} 2>/dev/null && [ \"$SECONDS\" -lt \"$limit\" ]; do sleep 0.01; done\nexec bash '{}' --finalize '{}' '{}'\n",
                actor.join("stop").display(),
                std::process::id(),
                captured.display(),
                id,
                fixture.runs.display()
            ),
        );
        let body = actor.join("selected-command");
        executable(
            &body,
            &format!(
                "#!/usr/bin/env bash\nset -eu\nprintf '%s\\n' \"$$\" >> '{}'\nwhile [ ! -f '{}' ]; do sleep 0.01; done\nexit \"$(cat '{}')\"\n",
                actor.join("body.pid").display(),
                actor.join("command.exit").display(),
                actor.join("command.exit").display()
            ),
        );
        let marker = expected;
        let output = fs::File::create(actor.join("controller.stdout"))?;
        let error = fs::File::create(actor.join("controller.stderr"))?;
        let barrier = fixture.temp.path().join("maintenance.lock");
        let mut command = Command::new("/usr/bin/systemd-run");
        let enclosure = fs::read_to_string("/proc/self/cgroup")?
            .split('/')
            .find(|part| {
                part.starts_with("overdrive-metal-run-") && part.trim().ends_with(".service")
            })
            .map(|part| part.trim().to_owned());
        if let Some(enclosure) = enclosure {
            command
                .arg(format!("--property=BindsTo={enclosure}"))
                .arg(format!("--property=After={enclosure}"));
        }
        command
            .args([
                "--quiet",
                "--pipe",
                "--wait",
                "--collect",
                "--expand-environment=no",
                "--property=Type=exec",
                "--property=KillMode=mixed",
                "--property=RuntimeMaxSec=60s",
                "--property=TimeoutStopSec=45s",
            ])
            .arg(format!("--unit=overdrive-metal-run-{id}.service"))
            .arg(format!("--property=ExecStopPost={}", gate.display()))
            .arg(format!("--setenv=PATH={}", fixture.path().to_string_lossy()))
            .arg(format!("--setenv=METAL_MODEL_ACTOR={}", actor.display()))
            .arg(format!("--setenv=METAL_MODEL_BARRIER={}", barrier.display()))
            .arg(format!("--setenv=METAL_MODEL_TREE={}", tree.display()))
            .arg("bash")
            .arg(&captured)
            .arg("--durable")
            .arg(&barrier)
            .arg(fixture.temp.path().join("maintenance.owner"))
            .args(["120", &id, "metal-protocol-conformance"])
            .arg(&marker["workspace"])
            .arg(&marker["commit"])
            .arg(&marker["source_digest"])
            .arg(source)
            .args(["root", "privileged"])
            .arg(&fixture.remote)
            .arg(fixture.path())
            .arg(&body)
            .arg(&fixture.runs)
            .arg(tree)
            .arg(tree.with_extension("owner"))
            .arg(class)
            .stdin(Stdio::piped())
            .stdout(output)
            .stderr(error);
        let child = command.spawn()?;
        Ok(Self {
            directory: fixture.runs.join(&id),
            id,
            actor,
            source: source.to_owned(),
            tree: tree.to_owned(),
            child,
            expected: expected.clone(),
        })
    }
    fn gate(&self, name: &str) -> Result {
        fs::write(self.actor.join(name), b"release\n")?;
        Ok(())
    }
    fn control(&mut self, line: &str) -> Result {
        writeln!(
            self.child.stdin.as_mut().ok_or_else(|| failed("controller already detached"))?,
            "{line}"
        )?;
        Ok(())
    }
    fn phase(&self) -> Option<String> {
        self.directory
            .join("phase.v1")
            .is_file()
            .then(|| fields(&self.directory.join("phase.v1"))["phase"].clone())
    }
    fn main_pid(&self) -> u32 {
        unit_facts(&self.id)
            .ok()
            .and_then(|f| f.get("MainPID").and_then(|p| p.parse().ok()))
            .unwrap_or(0)
    }
    fn ended(&self) -> Result {
        poll(
            || {
                unit_facts(&self.id).is_ok_and(|facts| {
                    live(&facts) && facts.get("MainPID").is_some_and(|pid| pid == "0")
                })
            },
            "main ended while actual stop owner remains live",
        )
    }
    fn terminal(&self) -> Result {
        poll(|| self.directory.join("terminal.v1").is_file(), "retained command/runner terminal")
    }
    fn stopped(&self) -> Result {
        poll(|| unit_facts(&self.id).is_ok_and(|f| !live(&f)), "actual systemd stop completion")
    }
}
impl Drop for NativeUnit {
    fn drop(&mut self) {
        for name in ["barrier", "checkout", "stop"] {
            let _ = self.gate(name);
        }
        let _ = fs::write(self.actor.join("command.exit"), b"0\n");
        drop(self.child.stdin.take());
        let _ = Command::new("/usr/bin/systemctl")
            .args(["stop", &format!("overdrive-metal-run-{}.service", self.id)])
            .status();
        let _ = self.child.wait();
        // Clear only a fixture-owned immutable blocker; never overwrite the
        // original produced failure receipt during fixture restoration.
        for root in [
            PathBuf::from("/srv/vm/metal").join(&self.id),
            PathBuf::from("/run/ovd-gk").join(&self.id),
            self.source.join("target/metal-run-tmp").join(&self.id),
        ] {
            let _ =
                Command::new("chattr").arg("-i").arg(root.join("model-immutable-input")).output();
            if root.is_dir() {
                let _ = fs::remove_dir_all(root);
            }
        }
    }
}

struct World {
    fixture: Fixture,
    locals: [PathBuf; 2],
    sources: [PathBuf; 2],
    selected_second: PathBuf,
    units: BTreeMap<i64, NativeUnit>,
    maintenance: Option<NativeUnit>,
    expected: [BTreeMap<String, String>; 2],
    // A single actual driving-port return, consumed by this step's comparison.
    collector_return: Option<(i64, Output)>,
}
impl World {
    fn new(second_tree: i64) -> Result<Self> {
        let fixture = Fixture::new();
        fs::remove_file(fixture.bin.join("systemctl"))?;
        executable(
            &fixture.bin.join("flock"),
            &text(&workspace_file("infra/metal/tests/fixtures/model-flock")),
        );
        let second = fixture.temp.path().join("second-local");
        success(&git(
            &fixture.source,
            &[
                "worktree",
                "add",
                "--detach",
                second.to_str().ok_or_else(|| failed("fixture path UTF-8"))?,
                "HEAD",
            ],
        ));
        success(&git(&second, &["symbolic-ref", "HEAD", "refs/heads/metal/isolation"]));
        let locals = [fixture.source.clone(), second];
        let mut roots = Vec::new();
        for local in &locals {
            fs::write(local.join("source-marker"), b"revision0\n")?;
            let before = fixture.bindings();
            success(&fixture.sync(local));
            let binding =
                fixture.bindings().into_iter().find(|path| !before.contains(path)).ok_or_else(
                    || failed("actual source transfer did not publish the canonical binding"),
                )?;
            roots.push(binding.parent().ok_or_else(|| failed("binding root"))?.to_owned());
        }
        let sources: [PathBuf; 2] =
            roots.try_into().map_err(|_| failed("two actual checkout owners required"))?;
        let selected_second = sources[usize::from(second_tree != 1)].clone();
        let expected =
            sources.each_ref().map(|source| fields(&source.join(".overdrive-metal-source")));
        Ok(Self {
            fixture,
            locals,
            sources,
            selected_second,
            units: BTreeMap::new(),
            maintenance: None,
            expected,
            collector_return: None,
        })
    }
    fn unit(&self, actor: i64) -> Result<&NativeUnit> {
        self.units.get(&actor).ok_or_else(|| failed("unregistered actor").into())
    }
    fn unit_mut(&mut self, actor: i64) -> Result<&mut NativeUnit> {
        self.units.get_mut(&actor).ok_or_else(|| failed("unregistered actor").into())
    }
    fn tree(&self, source: &Path) -> PathBuf {
        self.fixture.temp.path().join(if source == self.sources[0] {
            "first.lock"
        } else {
            "second.lock"
        })
    }
    fn register(&mut self, actor: i64) -> Result {
        let source = if actor == 1 { &self.sources[0] } else { &self.selected_second };
        let unit = NativeUnit::spawn(
            &self.fixture,
            source,
            &self.tree(source),
            &self.expected[usize::from(source != &self.sources[0])],
            &format!("actor-{actor}"),
            "ordinary",
        )?;
        poll(
            || {
                unit.directory.join("metadata.v2").is_file()
                    && unit.phase().as_deref() == Some("waiting_for_lease")
            },
            "v2 durable registration",
        )?;
        self.units.insert(actor, unit);
        Ok(())
    }
    fn barrier(&self, actor: i64) -> Result {
        let unit = self.unit(actor)?;
        unit.gate("barrier")?;
        poll(
            || {
                lock_mode(unit.main_pid(), &self.fixture.temp.path().join("maintenance.lock"))
                    .is_ok_and(|m| m.as_deref() == Some("shared"))
            },
            "actual shared admission descriptor",
        )
    }
    fn checkout(&self, actor: i64) -> Result {
        let unit = self.unit(actor)?;
        unit.gate("checkout")?;
        poll(
            || {
                unit.phase().as_deref() == Some("preparing")
                    && lock_mode(unit.main_pid(), &unit.tree)
                        .is_ok_and(|m| m.as_deref() == Some("exclusive"))
            },
            "actual checkout exclusion",
        )
    }
    fn source_transfer(&self, tree: i64, revision: u8, under_owned_preparation: bool) -> Result {
        let index = usize::try_from(tree - 1)?;
        fs::write(self.locals[index].join("source-marker"), format!("revision{revision}\n"))?;
        let mut command = self.fixture.bootstrap(&self.locals[index], &["--sync-only"]);
        // The same trusted controller already owns the durable checkout lock.
        // Invoke the real transfer adapter, not a second source implementation;
        // its local helper lock is confined to this controller's fixture domain.
        let lock = if under_owned_preparation {
            self.fixture.temp.path().join(format!("controller-{tree}.lock"))
        } else {
            self.tree(&self.sources[index])
        };
        command
            .env("OVERDRIVE_METAL_TREE_LOCK_PATH", &lock)
            .env("OVERDRIVE_METAL_TREE_OWNER_PATH", lock.with_extension("owner"));
        success(&command.output()?);
        fs::write(self.locals[index].join("source-marker"), b"revision0\n")?;
        Ok(())
    }
    fn preflight(unit: &NativeUnit, invalid_arch: bool) -> Result<Output> {
        let marker = &unit.expected;
        let mut command = Command::new("bash");
        command
            .arg(workspace_file("infra/metal/native-preflight.sh"))
            .env("OVERDRIVE_METAL_OWNER_PATH", unit.directory.join("owner.v1"))
            .env("OVERDRIVE_EXPECTED_TOKEN", &unit.id)
            .env("OVERDRIVE_EXPECTED_COMMIT", &marker["commit"])
            .env("OVERDRIVE_EXPECTED_WORKSPACE", &marker["workspace"])
            .env("OVERDRIVE_EXPECTED_SOURCE", &marker["source_digest"])
            .env("OVERDRIVE_REMOTE_DIR", &unit.source);
        if invalid_arch {
            command.env("OVERDRIVE_PREFLIGHT_ARCH", "aarch64");
        }
        Ok(command.output()?)
    }
    fn prepare(&self, actor: i64) -> Result {
        let unit = self.unit(actor)?;
        let tree = if unit.source == self.sources[0] { 1 } else { 2 };
        self.source_transfer(tree, 0, true)?;
        success(&Self::preflight(unit, false)?);
        Ok(())
    }
    fn start(&mut self, actor: i64) -> Result {
        let unit = self.unit_mut(actor)?;
        unit.control("START")?;
        poll(
            || unit.actor.join("body.pid").is_file() && unit.phase().as_deref() == Some("running"),
            "actual selected command process",
        )
    }
    fn lose_controller(&mut self, actor: i64) -> Result {
        let unit = self.unit_mut(actor)?;
        drop(unit.child.stdin.take());
        if unit.phase().as_deref() != Some("running") {
            unit.terminal()?;
            unit.ended()?;
        }
        Ok(())
    }
    fn abort(&mut self, actor: i64, value: i64) -> Result {
        if value == 78 {
            let output = Self::preflight(self.unit(actor)?, true)?;
            support::runner_error(&output, "preflight_failed", 78);
        }
        let unit = self.unit_mut(actor)?;
        unit.control(if value == 78 {
            "ABORT error=preflight_failed"
        } else {
            "ABORT error=runner_preparation_failed"
        })?;
        unit.terminal()?;
        unit.ended()
    }
    fn no_sync_refusal(&mut self, actor: i64) -> Result {
        let unit = self.unit_mut(actor)?;
        let actual_marker = fields(&unit.source.join(".overdrive-metal-source"));
        let actual_metadata = fields(&unit.directory.join("metadata.v2"));
        if text(&unit.source.join("source-marker")) != "revision1\n"
            || actual_marker["source_digest"] == actual_metadata["source_digest"]
        {
            return Err(
                failed("NoSyncRefusal requires actual mismatching transferred source").into()
            );
        }
        unit.control("ABORT error=source_mismatch")?;
        unit.terminal()?;
        unit.ended()?;
        let terminal = fields(&unit.directory.join("terminal.v1"));
        if terminal["kind"] != "runner"
            || terminal["error"] != "source_mismatch"
            || unit.actor.join("body.pid").exists()
        {
            return Err(failed("actual no-sync owner refusal diverged").into());
        }
        Ok(())
    }
    fn main_exit(&self, actor: i64, value: i64) -> Result {
        let unit = self.unit(actor)?;
        fs::write(unit.actor.join("command.exit"), format!("{value}\n"))?;
        unit.terminal()?;
        unit.ended()
    }
    fn supervisor_lost(&self, actor: i64) -> Result {
        let unit = self.unit(actor)?;
        // Existing systemd owner kills the actual main process; no forced Rust
        // future or fabricated missing terminal is involved.
        success(
            &Command::new("/usr/bin/systemctl")
                .args([
                    "kill",
                    "--kill-whom=main",
                    "--signal=KILL",
                    &format!("overdrive-metal-run-{}.service", unit.id),
                ])
                .output()?,
        );
        unit.ended()
    }
    fn admission_timeout(&self, actor: i64) -> Result {
        let unit = self.unit(actor)?;
        // From abstract phase 1 the real owner may acquire its shared barrier
        // before its existing deadline expires on checkout. Both actual timeout
        // causes project to the model's approved representative 75.
        let _holder = HeldLock::try_exclusive(&unit.tree, &unit.actor.join("blocker.held"))?;
        unit.gate("short-wait")?;
        unit.gate("barrier")?;
        unit.gate("checkout")?;
        unit.terminal()?;
        unit.ended()
    }
    fn stop_owner(&self, actor: i64, outcome: i64) -> Result {
        let unit = self.unit(actor)?;
        let blocker = PathBuf::from("/srv/vm/metal").join(&unit.id).join("model-immutable-input");
        if outcome == 2 {
            fs::create_dir_all(blocker.parent().ok_or_else(|| failed("blocker parent"))?)?;
            fs::write(&blocker, b"owned cleanup failure input")?;
            success(&Command::new("chattr").arg("+i").arg(&blocker).output()?);
        }
        if outcome == 0 {
            success(
                &Command::new("/usr/bin/systemctl")
                    .args([
                        "kill",
                        "--kill-whom=control",
                        "--signal=KILL",
                        &format!("overdrive-metal-run-{}.service", unit.id),
                    ])
                    .output()?,
            );
        } else {
            unit.gate("stop")?;
        }
        unit.stopped()?;
        if outcome == 3 {
            let path = unit.directory.join("completion.v1");
            let original = fs::read(&path)?;
            fs::write(unit.actor.join("original-completion-before-corruption"), original)?;
            fs::remove_file(path)?;
        }
        let cgroup = unit_facts(&unit.id)?.get("ControlGroup").cloned().unwrap_or_default();
        if !cgroup.is_empty() {
            let root = PathBuf::from("/sys/fs/cgroup").join(cgroup.trim_start_matches('/'));
            if root.join("cgroup.procs").is_file()
                && !text(&root.join("cgroup.procs")).trim().is_empty()
            {
                return Err(failed("stopped owner retains actual cgroup processes").into());
            }
        }
        if outcome == 1 || outcome == 3 {
            for root in [
                PathBuf::from("/srv/vm/metal").join(&unit.id),
                PathBuf::from("/run/ovd-gk").join(&unit.id),
                unit.source.join("target/metal-run-tmp").join(&unit.id),
            ] {
                if fs::symlink_metadata(root).is_ok() {
                    return Err(failed("actual completed owner retains an ephemeral root").into());
                }
            }
        }
        Ok(())
    }
    fn collect(&mut self, actor: i64) -> Result {
        let id = self.unit(actor)?.id.clone();
        let before = snapshot(&self.fixture.runs);
        let output = self.fixture.collect(&id).output()?;
        if snapshot(&self.fixture.runs) != before {
            return Err(failed("collector rewrote retained owner state").into());
        }
        self.collector_return = Some((actor, output));
        Ok(())
    }
    fn spawn_maintainer(&self) -> Result<NativeUnit> {
        // The maintenance caller selects the actual current bound revision;
        // Maintain does not reset the model's source-revision input.
        let marker = fields(&self.sources[0].join(".overdrive-metal-source"));
        let local = self.locals[0].join("source-marker");
        let original = fs::read(&local)?;
        fs::copy(self.sources[0].join("source-marker"), &local)?;
        let result = NativeUnit::spawn(
            &self.fixture,
            &self.sources[0],
            &self.fixture.temp.path().join("maintainer.lock"),
            &marker,
            "maintainer",
            "exclusive",
        );
        fs::write(local, original)?;
        result
    }
    fn maintain(&mut self, value: i64) -> Result {
        let barrier = self.fixture.temp.path().join("maintenance.lock");
        if value == 0 {
            let _holder = HeldLock::shared(
                &barrier,
                &self.fixture.temp.path().join("maintenance-blocker.held"),
            )?;
            let unit = self.spawn_maintainer()?;
            unit.gate("short-wait")?;
            unit.gate("barrier")?;
            unit.gate("checkout")?;
            unit.terminal()?;
            let terminal = fields(&unit.directory.join("terminal.v1"));
            if terminal["kind"] != "runner" || terminal["error"] != "maintenance_timeout" {
                return Err(failed("actual maintainer timeout changed its typed cause").into());
            }
            unit.gate("stop")?;
            unit.stopped()?;
            self.maintenance = Some(unit);
        } else if value == 1 {
            let unit = self.spawn_maintainer()?;
            unit.gate("barrier")?;
            unit.gate("checkout")?;
            poll(
                || {
                    lock_mode(unit.main_pid(), &barrier)
                        .is_ok_and(|mode| mode.as_deref() == Some("exclusive"))
                },
                "exclusive maintenance descriptor",
            )?;
            self.maintenance = Some(unit);
        } else if value == 2 {
            let unit =
                self.maintenance.as_ref().ok_or_else(|| failed("no actual maintenance owner"))?;
            poll(
                || unit.phase().as_deref() == Some("preparing"),
                "maintenance waits for admitted unit stop",
            )?;
            success(&Self::preflight(unit, false)?);
            let unit = self.maintenance.as_mut().ok_or_else(|| failed("no maintenance owner"))?;
            unit.control("START")?;
            poll(|| unit.actor.join("body.pid").is_file(), "actual maintenance command start")?;
        } else {
            let unit = self.maintenance.as_ref().ok_or_else(|| failed("no maintenance owner"))?;
            fs::write(unit.actor.join("command.exit"), b"0\n")?;
            unit.terminal()?;
            unit.gate("stop")?;
            unit.stopped()?;
        }
        Ok(())
    }
    fn idle(&self) -> Result {
        for unit in self.units.values() {
            let _ = unit_facts(&unit.id)?;
        }
        Ok(())
    }
}

impl Drop for World {
    fn drop(&mut self) {
        drop(self.maintenance.take());
        self.units.clear();
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RunState {
    p: i64,
    term: i64,
    receipt: i64,
}
#[derive(Debug)]
enum Collection {
    None,
    Expected(i64, i64),
    Actual(i64, i32),
}
#[derive(Debug)]
struct Projection {
    runs: BTreeMap<i64, RunState>,
    second_tree: i64,
    source_revision: BTreeMap<i64, i64>,
    maintenance: i64,
    collection: Collection,
}
impl PartialEq for Projection {
    fn eq(&self, other: &Self) -> bool {
        let collection = match (&self.collection, &other.collection) {
            (Collection::None, Collection::None) => true,
            (Collection::Expected(a, x), Collection::Expected(b, y)) => a == b && x == y,
            (Collection::Actual(a, x), Collection::Actual(b, y)) => a == b && x == y,
            (Collection::Expected(actor, expected), Collection::Actual(actual_actor, actual))
            | (Collection::Actual(actual_actor, actual), Collection::Expected(actor, expected)) => {
                actor == actual_actor && *expected == i64::from(*actual)
            }
            _ => false,
        };
        self.runs == other.runs
            && self.second_tree == other.second_tree
            && self.source_revision == other.source_revision
            && self.maintenance == other.maintenance
            && collection
    }
}
fn integer(value: &Value) -> Result<i64> {
    match value {
        Value::Number(n) => Ok(*n),
        Value::BigInt(n) => Ok(i64::try_from(n.get().clone())?),
        _ => Err(failed("expected bounded model integer").into()),
    }
}
fn record(value: &Value) -> Result<&itf::value::Record> {
    if let Value::Record(record) = value {
        Ok(record)
    } else {
        Err(failed("expected model record").into())
    }
}
fn var<'a>(value: &'a Value, name: &str) -> Result<&'a Value> {
    record(value)?
        .get(&format!("runner_conf::runner_protocol::{name}"))
        .ok_or_else(|| failed(format!("missing model variable {name}")).into())
}
fn map(value: &Value) -> Result<Vec<(i64, &Value)>> {
    if let Value::Map(values) = value {
        values.iter().map(|(key, value)| Ok((integer(key)?, value))).collect()
    } else {
        Err(failed("expected model map").into())
    }
}
impl Projection {
    fn from_value(value: &Value) -> Result<Self> {
        let runs = map(var(value, "runs")?)?
            .into_iter()
            .map(|(id, value)| {
                let row = record(value)?;
                Ok((
                    id,
                    RunState {
                        p: integer(row.get("p").ok_or_else(|| failed("missing model phase"))?)?,
                        term: integer(
                            row.get("term").ok_or_else(|| failed("missing model terminal"))?,
                        )?,
                        receipt: integer(
                            row.get("receipt").ok_or_else(|| failed("missing model receipt"))?,
                        )?,
                    },
                ))
            })
            .collect::<Result<_>>()?;
        let source_revision = map(var(value, "source_revision")?)?
            .into_iter()
            .map(|(id, value)| Ok((id, integer(value)?)))
            .collect::<Result<_>>()?;
        let action = record(var(value, "conf_step")?)?;
        let collection = if action.get("tag") == Some(&Value::String("Collect".into())) {
            let actor = integer(
                record(action.get("value").ok_or_else(|| failed("missing action value"))?)?
                    .get("actor")
                    .ok_or_else(|| failed("missing action actor"))?,
            )?;
            let observed = map(var(value, "observed")?)?
                .into_iter()
                .find(|(id, _)| *id == actor)
                .ok_or_else(|| failed("missing this collection's model result"))?;
            Collection::Expected(actor, integer(observed.1)?)
        } else {
            Collection::None
        };
        Ok(Self {
            runs,
            second_tree: integer(var(value, "second_tree")?)?,
            source_revision,
            maintenance: integer(var(value, "maintenance")?)?,
            collection,
        })
    }
}
impl<'de> Deserialize<'de> for Projection {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        Self::from_value(&Value::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
struct RunnerDriver {
    owner: Option<World>,
}
impl RunnerDriver {
    const fn fresh() -> Self {
        Self { owner: None }
    }
    fn owner(&self) -> Result<&World> {
        self.owner.as_ref().ok_or_else(|| failed("missing actual owner composition").into())
    }
    fn owner_mut(&mut self) -> Result<&mut World> {
        self.owner.as_mut().ok_or_else(|| failed("missing actual owner composition").into())
    }
}
impl Driver for RunnerDriver {
    type State = Projection;
    fn config() -> Config {
        Config { state: &[], nondet: &["runner_conf::runner_protocol::conf_step"] }
    }
    fn step(&mut self, step: &Step) -> Result {
        if let Some(world) = &mut self.owner {
            world.collector_return = None;
        }
        switch!(step {
            init(actor:i64,value:i64)=> {if actor!=0{return Err(failed("invalid initial actor").into());}self.owner=Some(World::new(value)?);},
            Register(actor:i64,value:i64)=> {let _=value;self.owner_mut()?.register(actor)?;},
            Barrier(actor:i64,value:i64)=> {let _=value;self.owner()?.barrier(actor)?;},
            Checkout(actor:i64,value:i64)=> {let _=value;self.owner()?.checkout(actor)?;},
            Prepare(actor:i64,value:i64)=> {let _=value;self.owner()?.prepare(actor)?;},
            Start(actor:i64,value:i64)=> {let _=value;self.owner_mut()?.start(actor)?;},
            LoseController(actor:i64,value:i64)=> {let _=value;self.owner_mut()?.lose_controller(actor)?;},
            AdmissionTimeout(actor:i64,value:i64)=> {let _=value;self.owner()?.admission_timeout(actor)?;},
            AbortPreparation(actor:i64,value:i64)=> self.owner_mut()?.abort(actor,value)?,
            MainExit(actor:i64,value:i64)=> self.owner()?.main_exit(actor,value)?,
            SupervisorLost(actor:i64,value:i64)=> {let _=value;self.owner()?.supervisor_lost(actor)?;},
            StopOwner(actor:i64,value:i64)=> self.owner()?.stop_owner(actor,value)?,
            Collect(actor:i64,value:i64)=> {let _=value;self.owner_mut()?.collect(actor)?;},
            Sync(actor:i64,value:i64)=> {if value!=1{return Err(failed("invalid sync revision stimulus").into());}self.owner()?.source_transfer(actor,1,false)?;},
            NoSyncRefusal(actor:i64,value:i64)=> {let _=value;self.owner_mut()?.no_sync_refusal(actor)?;},
            Maintain(actor:i64,value:i64)=> {if actor!=0{return Err(failed("invalid maintainer actor").into());}self.owner_mut()?.maintain(value)?;},
            Idle(actor:i64,value:i64)=> {let _=(actor,value);self.owner()?.idle()?;},
        })
    }
}
impl State<RunnerDriver> for Projection {
    fn from_spec(value: Value) -> Result<Self> {
        Self::from_value(&value)
    }
    fn from_driver(driver: &RunnerDriver) -> Result<Self> {
        let world = driver.owner()?;
        let barrier = world.fixture.temp.path().join("maintenance.lock");
        let mut runs = BTreeMap::new();
        for actor in [1, 2] {
            let input_actor = world.fixture.temp.path().join(format!("actor-{actor}"));
            let input_id =
                format!("{:x}", Sha256::digest(input_actor.as_os_str().as_encoded_bytes()))[..24]
                    .to_owned();
            let directory = world.fixture.runs.join(&input_id);
            let absent_facts = unit_facts(&input_id)?;
            if !directory.exists() && !live(&absent_facts) {
                runs.insert(actor, RunState { p: 0, term: -1, receipt: 0 });
                continue;
            }
            let unit = world
                .units
                .get(&actor)
                .ok_or_else(|| failed("an actual owner exists without its driving handle"))?;
            let facts = unit_facts(&unit.id)?;
            let pid = facts.get("MainPID").and_then(|p| p.parse::<u32>().ok()).unwrap_or(0);
            let metadata = validated(
                &unit.directory.join("metadata.v2"),
                &[
                    "format",
                    "run_id",
                    "token",
                    "unit",
                    "created_at",
                    "scenario_b64",
                    "workspace_b64",
                    "commit",
                    "source_digest",
                    "remote_dir_b64",
                    "target_user_b64",
                    "execution_mode",
                    "execution_class",
                ],
                Some("overdrive-metal-run-v2"),
            )?;
            if metadata["run_id"] != unit.id
                || metadata["execution_class"] != "ordinary"
                || unit.directory.join("metadata.v1").exists()
            {
                return Err(failed("invalid registered owner identity/class/version").into());
            }
            let phase = validated(
                &unit.directory.join("phase.v1"),
                &["format", "run_id", "phase", "updated_at"],
                Some("overdrive-metal-phase-v1"),
            )?;
            let p = if !live(&facts) {
                6
            } else if pid == 0 {
                5
            } else if phase["phase"] == "running" {
                4
            } else if phase["phase"] == "preparing"
                && lock_mode(pid, &unit.tree)?.as_deref() == Some("exclusive")
            {
                3
            } else if lock_mode(pid, &barrier)?.as_deref() == Some("shared") {
                2
            } else {
                1
            };
            if p == 4 {
                let body_pids = text(&unit.actor.join("body.pid"));
                let values: Vec<_> = body_pids.lines().collect();
                if values.len() != 1 {
                    return Err(failed("actual selected command did not start exactly once").into());
                }
                let selected_pid: u32 = values[0].parse()?;
                let cgroup = fs::read_to_string(format!("/proc/{selected_pid}/cgroup"))?;
                let owner_group = facts
                    .get("ControlGroup")
                    .ok_or_else(|| failed("missing actual owner cgroup"))?;
                if !cgroup.lines().any(|line| {
                    line.strip_prefix("0::").is_some_and(|path| {
                        path == owner_group || path.starts_with(&format!("{owner_group}/"))
                    })
                }) {
                    return Err(
                        failed("selected command is not alive in its actual owner cgroup").into()
                    );
                }
            }
            let terminal_path = unit.directory.join("terminal.v1");
            let term = if terminal_path.exists() {
                let row = fields(&terminal_path);
                if row["kind"] == "command" {
                    let row = validated(
                        &terminal_path,
                        &["format", "run_id", "kind", "exit_code", "cause", "finished_at"],
                        Some("overdrive-metal-terminal-v1"),
                    )?;
                    let code: i64 = row["exit_code"].parse()?;
                    if !(0..=255).contains(&code) {
                        return Err(failed("invalid command code").into());
                    }
                    code
                } else {
                    let row = validated(
                        &terminal_path,
                        &["format", "run_id", "kind", "error", "finished_at"],
                        Some("overdrive-metal-terminal-v1"),
                    )?;
                    match row["error"].as_str() {
                        "checkout_identity_conflict" | "source_mismatch" | "preflight_failed" => 78,
                        "maintenance_timeout"
                        | "checkout_timeout"
                        | "runner_preparation_failed"
                        | "launch_aborted"
                        | "controller_eof_before_start" => 75,
                        _ => {
                            return Err(failed(
                                "runner error outside the protocol's approved representative set",
                            )
                            .into());
                        }
                    }
                }
            } else {
                -1
            };
            let receipt_path = unit.directory.join("completion.v1");
            let receipt = if receipt_path.exists() {
                let row = fields(&receipt_path);
                if row["kind"] == "complete" {
                    validated(
                        &receipt_path,
                        &["format", "run_id", "kind", "finished_at"],
                        Some("overdrive-metal-completion-v1"),
                    )?;
                    1
                } else {
                    let row = validated(
                        &receipt_path,
                        &["format", "run_id", "kind", "error", "root", "finished_at"],
                        Some("overdrive-metal-completion-v1"),
                    )?;
                    if row["error"] != "cleanup_failed"
                        || !matches!(
                            row["root"].as_str(),
                            "guest_staging" | "vmm_runtime" | "command_tmp"
                        )
                    {
                        return Err(failed("invalid actual cleanup failure").into());
                    }
                    2
                }
            } else {
                0
            };
            runs.insert(actor, RunState { p, term, receipt });
        }
        let first_binding = validated(
            &world.sources[0].join(".overdrive-metal-checkout.v1"),
            &["format", "kind", "checkout_key", "identity_b64"],
            Some("overdrive-metal-checkout-v1"),
        )?;
        let second_binding = validated(
            &world.selected_second.join(".overdrive-metal-checkout.v1"),
            &["format", "kind", "checkout_key", "identity_b64"],
            Some("overdrive-metal-checkout-v1"),
        )?;
        let second_tree = if first_binding == second_binding
            && fs::canonicalize(&world.sources[0])? == fs::canonicalize(&world.selected_second)?
        {
            1
        } else {
            2
        };
        let mut source_revision = BTreeMap::new();
        for (index, source) in world.sources.iter().enumerate() {
            let marker = validated(
                &source.join(".overdrive-metal-source"),
                &["commit", "workspace", "source_digest"],
                None,
            )?;
            if marker["source_digest"].len() != 64 {
                return Err(failed("invalid actual source digest").into());
            }
            let revision = match text(&source.join("source-marker")).as_str() {
                "revision0\n" => 0,
                "revision1\n" => 1,
                _ => return Err(failed("transferred bytes outside controlled revisions").into()),
            };
            source_revision.insert(i64::try_from(index)? + 1, revision);
        }
        let maintenance = if let Some(unit) = &world.maintenance {
            let facts = unit_facts(&unit.id)?;
            if !live(&facts) {
                3
            } else if unit.actor.join("body.pid").is_file() {
                2
            } else {
                i64::from(lock_mode(unit.main_pid(), &barrier)?.as_deref() == Some("exclusive"))
            }
        } else {
            let actor = world.fixture.temp.path().join("maintainer");
            let id = format!("{:x}", Sha256::digest(actor.as_os_str().as_encoded_bytes()))[..24]
                .to_owned();
            if live(&unit_facts(&id)?) || world.fixture.runs.join(id).exists() {
                return Err(failed("unexpected actual maintainer without a handle").into());
            }
            0
        };
        // This is one actual return from the collector driving port, not the
        // model-only observed map, a predicted status, or a result ledger.
        let collection = if let Some((actor, output)) = &world.collector_return {
            Collection::Actual(
                *actor,
                output.status.code().ok_or_else(|| failed("collector ended without a status"))?,
            )
        } else {
            Collection::None
        };
        Ok(Self { runs, second_tree, source_revision, maintenance, collection })
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../specs/quint/metal-run-isolation/runner_protocol.qnt",
    main = "runner_conf",
    test = "independent_worktrees",
    max_samples = 1
)]
#[ignore = "pending DELIVER: metal-run-isolation real-owner protocol conformance"]
fn protocol_independent_worktrees() -> impl Driver {
    RunnerDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../specs/quint/metal-run-isolation/runner_protocol.qnt",
    main = "runner_conf",
    test = "same_checkout_excludes_sync",
    max_samples = 1
)]
#[ignore = "pending DELIVER: metal-run-isolation real-owner protocol conformance"]
fn protocol_same_checkout_excludes_sync() -> impl Driver {
    RunnerDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../specs/quint/metal-run-isolation/runner_protocol.qnt",
    main = "runner_conf",
    test = "controller_eof_before_start",
    max_samples = 1
)]
#[ignore = "pending DELIVER: metal-run-isolation real-owner protocol conformance"]
fn protocol_controller_eof_before_start() -> impl Driver {
    RunnerDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../specs/quint/metal-run-isolation/runner_protocol.qnt",
    main = "runner_conf",
    test = "controller_loss_after_start",
    max_samples = 1
)]
#[ignore = "pending DELIVER: metal-run-isolation real-owner protocol conformance"]
fn protocol_controller_loss_after_start() -> impl Driver {
    RunnerDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../specs/quint/metal-run-isolation/runner_protocol.qnt",
    main = "runner_conf",
    test = "supervisor_loss_is_not_a_result",
    max_samples = 1
)]
#[ignore = "pending DELIVER: metal-run-isolation real-owner protocol conformance"]
fn protocol_supervisor_loss_is_not_a_result() -> impl Driver {
    RunnerDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../specs/quint/metal-run-isolation/runner_protocol.qnt",
    main = "runner_conf",
    test = "cleanup_failure_keeps_command",
    max_samples = 1
)]
#[ignore = "pending DELIVER: metal-run-isolation real-owner protocol conformance"]
fn protocol_cleanup_failure_keeps_command() -> impl Driver {
    RunnerDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../specs/quint/metal-run-isolation/runner_protocol.qnt",
    main = "runner_conf",
    test = "missing_completion_refuses",
    max_samples = 1
)]
#[ignore = "pending DELIVER: metal-run-isolation real-owner protocol conformance"]
fn protocol_missing_completion_refuses() -> impl Driver {
    RunnerDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../specs/quint/metal-run-isolation/runner_protocol.qnt",
    main = "runner_conf",
    test = "maintenance_waits_for_stop",
    max_samples = 1
)]
#[ignore = "pending DELIVER: metal-run-isolation real-owner protocol conformance"]
fn protocol_maintenance_waits_for_stop() -> impl Driver {
    RunnerDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../specs/quint/metal-run-isolation/runner_protocol.qnt",
    main = "runner_conf",
    test = "no_sync_mismatch_refuses",
    max_samples = 1
)]
#[ignore = "pending DELIVER: metal-run-isolation real-owner protocol conformance"]
fn protocol_no_sync_mismatch_refuses() -> impl Driver {
    RunnerDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../specs/quint/metal-run-isolation/runner_protocol.qnt",
    main = "runner_conf",
    test = "admission_timeout",
    max_samples = 1
)]
#[ignore = "pending DELIVER: metal-run-isolation real-owner protocol conformance"]
fn protocol_admission_timeout() -> impl Driver {
    RunnerDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../specs/quint/metal-run-isolation/runner_protocol.qnt",
    main = "runner_conf",
    test = "preflight_aborts_before_start",
    max_samples = 1
)]
#[ignore = "pending DELIVER: metal-run-isolation real-owner protocol conformance"]
fn protocol_preflight_aborts_before_start() -> impl Driver {
    RunnerDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../specs/quint/metal-run-isolation/runner_protocol.qnt",
    main = "runner_conf",
    test = "missing_rsync_aborts_before_start",
    max_samples = 1
)]
#[ignore = "pending DELIVER: metal-run-isolation real-owner protocol conformance"]
fn protocol_missing_rsync_aborts_before_start() -> impl Driver {
    RunnerDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../specs/quint/metal-run-isolation/runner_protocol.qnt",
    main = "runner_conf",
    test = "repeat_collection_agrees",
    max_samples = 1
)]
#[ignore = "pending DELIVER: metal-run-isolation real-owner protocol conformance"]
fn protocol_repeat_collection_agrees() -> impl Driver {
    RunnerDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_run(
    spec = "../specs/quint/metal-run-isolation/runner_protocol.qnt",
    main = "runner_conf",
    max_samples = 4,
    max_steps = 24
)]
#[ignore = "pending DELIVER: metal-run-isolation seeded real-owner protocol conformance"]
fn protocol_randomized_owner_traces() -> impl Driver {
    RunnerDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: metal-run-isolation controls across actual owner states"]
fn protocol_illegal_controls_preserve_each_actual_owner_state_boundary() -> Result {
    // No addressable unit exists before registration; retrieval is the actual
    // public operation and must preserve absence with its exact typed refusal.
    let mut world = World::new(2)?;
    let actor = world.fixture.temp.path().join("actor-1");
    let id = format!("{:x}", Sha256::digest(actor.as_os_str().as_encoded_bytes()))[..24].to_owned();
    support::runner_error(&world.fixture.collect(&id).output()?, "run_not_found", 66);
    for stage in [1, 2, 3] {
        drop(world);
        world = World::new(2)?;
        world.register(1)?;
        if stage >= 2 {
            world.barrier(1)?;
        }
        if stage >= 3 {
            world.checkout(1)?;
        }
        let unit = world.unit_mut(1)?;
        unit.control("START extra")?;
        unit.gate("barrier")?;
        unit.gate("checkout")?;
        unit.terminal()?;
        unit.ended()?;
        let terminal = fields(&unit.directory.join("terminal.v1"));
        if terminal["kind"] != "runner"
            || terminal["error"] != "launch_aborted"
            || unit.actor.join("body.pid").exists()
        {
            return Err(failed("illegal prestart control executed a body or changed cause").into());
        }
        unit.gate("stop")?;
        unit.stopped()?;
    }
    drop(world);
    let mut world = World::new(2)?;
    world.register(1)?;
    world.barrier(1)?;
    world.checkout(1)?;
    world.prepare(1)?;
    world.start(1)?;
    let initial_body = fs::read(world.unit(1)?.actor.join("body.pid"))?;
    let _ = world.unit_mut(1)?.control("START"); // detached running controller
    world.main_exit(1, 0)?;
    let terminal = fs::read(world.unit(1)?.directory.join("terminal.v1"))?;
    let _ = world.unit_mut(1)?.control("START"); // terminal, real unit still stopping
    if fs::read(world.unit(1)?.directory.join("terminal.v1"))? != terminal {
        return Err(failed("late control replaced terminal").into());
    }
    world.stop_owner(1, 1)?;
    let _ = world.unit_mut(1)?.control("START"); // stopped, no addressable main process
    if fs::read(world.unit(1)?.actor.join("body.pid"))? != initial_body {
        return Err(failed("control replay executed another selected command").into());
    }
    world.collect(1)?;
    let first =
        world.collector_return.take().ok_or_else(|| failed("missing actual collector return"))?.1;
    world.collect(1)?;
    let second = world
        .collector_return
        .take()
        .ok_or_else(|| failed("missing repeat actual collector return"))?
        .1;
    if first.status.code() != Some(0)
        || second.status.code() != Some(0)
        || first.stdout != second.stdout
        || first.stderr != second.stderr
    {
        return Err(failed("stopped-owner read-only repeat collection diverged").into());
    }
    let before = snapshot(&world.fixture.runs);
    support::runner_error(&world.fixture.collect("invalid").output()?, "invalid_run_id", 64);
    if snapshot(&world.fixture.runs) != before {
        return Err(failed("invalid post-stop retrieval changed records").into());
    }
    Ok(())
}
