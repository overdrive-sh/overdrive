use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

pub const RUN_A: &str = "0123456789abcdef01234567";
pub const RUN_B: &str = "89abcdef0123456789abcdef";
pub const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
pub const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
pub const NOW: &str = "2026-11-08T12:00:00Z";

pub fn workspace_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(relative)
}

pub fn text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

pub fn executable(path: &Path, body: &str) {
    fs::write(path, body).expect("write fixture executable");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod fixture");
}

pub fn success(output: &Output) {
    assert!(
        output.status.success(),
        "status {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn runner_error(output: &Output, code: &str, status: i32) {
    assert_eq!(output.status.code(), Some(status), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains(&format!("OVERDRIVE_METAL_RUNNER_ERROR error={code}")),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("OVERDRIVE_METAL_RUN_COMPLETE"));
}

pub fn test_error(output: &Output, code: &str, status: i32) {
    assert_eq!(output.status.code(), Some(status), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains(&format!("OVERDRIVE_METAL_TEST_ERROR error={code}")),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn fields(path: &Path) -> BTreeMap<String, String> {
    let content = text(path);
    assert!(content.ends_with('\n'), "record must be LF terminated");
    let mut result = BTreeMap::new();
    for line in content.lines() {
        let (key, value) = line.split_once('=').expect("record key=value");
        assert!(result.insert(key.into(), value.into()).is_none(), "duplicate key {key}");
    }
    result
}

pub fn b64(bytes: &[u8]) -> String {
    let mut child = Command::new("base64")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("base64");
    child.stdin.take().expect("base64 stdin").write_all(bytes).expect("base64 input");
    let output = child.wait_with_output().expect("base64 output");
    success(&output);
    String::from_utf8(output.stdout).expect("base64 ASCII").lines().collect()
}

pub fn git(root: &Path, args: &[&str]) -> Output {
    Command::new("git").arg("-C").arg(root).args(args).output().expect("git fixture")
}

pub fn git_text(root: &Path, args: &[&str]) -> String {
    let output = git(root, args);
    success(&output);
    String::from_utf8(output.stdout).expect("Git UTF-8 fixture").trim_end().to_owned()
}

pub fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(base: &Path, at: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        if !at.exists() {
            return;
        }
        for entry in fs::read_dir(at).expect("snapshot directory") {
            let path = entry.expect("snapshot entry").path();
            let metadata = fs::symlink_metadata(&path).expect("lstat snapshot");
            if metadata.is_dir() {
                walk(base, &path, result);
            } else if metadata.is_symlink() {
                result.insert(
                    path.strip_prefix(base).expect("snapshot relative").to_owned(),
                    fs::read_link(&path).expect("readlink").as_os_str().as_encoded_bytes().to_vec(),
                );
            } else if metadata.is_file() {
                result.insert(
                    path.strip_prefix(base).expect("snapshot relative").to_owned(),
                    fs::read(&path).expect("snapshot bytes"),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    walk(root, root, &mut result);
    result
}

pub struct Fixture {
    pub temp: TempDir,
    pub source: PathBuf,
    pub remote: PathBuf,
    pub bin: PathBuf,
    pub runs: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        Self::with_temp(TempDir::new().expect("metal test universe"))
    }
    pub fn new_accessible() -> Self {
        // A nested user namespace cannot use init-user-namespace DAC override
        // to traverse a login user's private home. Keep its owned test inputs
        // below the already-prepared uid-0 runtime root when Metal supplies one.
        let base = std::env::var_os("OVERDRIVE_METAL_RUN_ID").map_or_else(
            || PathBuf::from("/tmp"),
            |id| {
                let id = id.to_string_lossy();
                assert!(
                    id.len() == 24
                        && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                    "valid enclosing service context"
                );
                let root = PathBuf::from("/run/ovd-gk").join(id.as_ref());
                assert!(
                    root.is_dir(),
                    "prepare the enclosing P3 runtime context before activating namespace fixtures"
                );
                root
            },
        );
        Self::with_temp(TempDir::new_in(base).expect("owned traversable namespace fixture"))
    }
    #[expect(
        clippy::literal_string_with_formatting_args,
        reason = "Bash parameter expansion in a fixture script"
    )]
    fn with_temp(temp: TempDir) -> Self {
        let source = temp.path().join("source");
        let remote = temp.path().join("remote");
        let bin = temp.path().join("bin");
        let runs = temp.path().join("runs");
        for path in [&source, &remote, &bin, &runs] {
            fs::create_dir_all(path).expect("fixture directory");
        }
        fs::create_dir_all(source.join("infra/metal")).expect("script directory");
        for name in [
            "bootstrap.sh",
            "lease-holder.sh",
            "native-preflight.sh",
            "test-wrapper.sh",
            "test-placement.json",
        ] {
            let original = workspace_file(&format!("infra/metal/{name}"));
            if original.is_file() {
                fs::copy(original, source.join("infra/metal").join(name))
                    .expect("copy current runner");
            }
        }
        for name in ["ssh", "rsync", "systemctl"] {
            executable(
                &bin.join(name),
                &text(&workspace_file(&format!("infra/metal/tests/fixtures/{name}"))),
            );
        }
        executable(
            &bin.join("sudo"),
            "#!/usr/bin/env bash\nset -eu\n[ \"${1:-}\" != -n ] || shift\nexec \"$@\"\n",
        );
        fs::write(temp.path().join("unit-state"), "inactive\n").expect("unit state");
        fs::write(source.join("source-marker"), "first-worktree\n").expect("contrasting input");
        success(&git(&source, &["init", "-b", "metal/isolation"]));
        success(&git(&source, &["add", "."]));
        success(&git(
            &source,
            &[
                "-c",
                "user.name=Metal fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-m",
                "Fixture source\n\nCo-Authored-By: Codex <codex@openai.com>",
            ],
        ));
        Self { temp, source, remote, bin, runs }
    }
    pub fn path(&self) -> std::ffi::OsString {
        std::env::join_paths(
            std::iter::once(self.bin.clone())
                .chain(std::env::split_paths(&std::env::var_os("PATH").expect("PATH"))),
        )
        .expect("fixture PATH")
    }
    pub fn context_id(&self) -> String {
        use sha2::{Digest as _, Sha256};
        format!(
            "{:x}",
            Sha256::digest(
                [self.temp.path().as_os_str().as_encoded_bytes(), b"exec-context"].concat()
            )
        )[..24]
            .to_owned()
    }
    pub fn bound_context_inputs(&self) {
        use sha2::{Digest as _, Sha256};
        let mut identity = fs::canonicalize(&self.source)
            .expect("physical context source")
            .as_os_str()
            .as_encoded_bytes()
            .to_vec();
        identity.push(0);
        identity.extend(git_text(&self.source, &["symbolic-ref", "HEAD"]).bytes());
        let key = format!(
            "{:x}",
            Sha256::digest(
                [b"overdrive-metal-checkout-v1\0branch\0".as_slice(), &identity].concat()
            )
        );
        fs::write(self.source.join(".overdrive-metal-checkout.v1"),format!("format=overdrive-metal-checkout-v1\nkind=branch\ncheckout_key={key}\nidentity_b64={}\n",b64(&identity))).expect("already-bound service context input");
        fs::write(
            self.source.join(".overdrive-metal-source"),
            format!(
                "commit={}\nworkspace={}\nsource_digest={DIGEST}\n",
                git_text(&self.source, &["rev-parse", "HEAD"]),
                self.source.display()
            ),
        )
        .expect("already-prepared source input");
    }
    pub fn environment(&self, command: &mut Command) {
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if name.starts_with("NEXTEST_BIN_EXE_") || name.starts_with("CARGO_BIN_EXE_") {
                command.env_remove(key);
            }
        }
        for key in [
            "CARGO_TARGET_DIR",
            "OVERDRIVE_BPF_OBJECT",
            "OVERDRIVE_KMOD_DIR",
            "OVERDRIVE_METAL_RUN_ID",
            "OVERDRIVE_METAL_EXECUTION_CLASS",
            "OVERDRIVE_METAL_SOURCE_ROOT",
            "OVERDRIVE_METAL_TEST_DISPATCHER",
        ] {
            command.env_remove(key);
        }
        command
            .env("PATH", self.path())
            .env("RSYNC_BIN", self.bin.join("rsync"))
            .env("METAL_FIXTURE_REMOTE_HOME", &self.remote)
            .env("METAL_FIXTURE_SSH_LOG", self.temp.path().join("ssh.log"))
            .env("METAL_FIXTURE_FORBIDDEN_INSTALL", self.temp.path().join("forbidden-install"))
            .env("METAL_FIXTURE_UNIT_STATE", self.temp.path().join("unit-state"))
            .env("OVERDRIVE_METAL_LOCK_PATH", self.temp.path().join("maintenance.lock"))
            .env("OVERDRIVE_METAL_OWNER_PATH", self.temp.path().join("maintenance.owner"))
            .env("OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS", "1")
            .env_remove("OVERDRIVE_METAL_REMOTE_SUBDIR")
            .env("OVERDRIVE_METAL_TREE_LOCK_PATH", self.temp.path().join("fixture-tree.lock"))
            .env("OVERDRIVE_METAL_TREE_OWNER_PATH", self.temp.path().join("fixture-tree.owner"));
    }
    pub fn bootstrap(&self, source: &Path, args: &[&str]) -> Command {
        let mut command = Command::new("timeout");
        command
            .args(["--kill-after=2s", "15s", "bash"])
            .arg(source.join("infra/metal/bootstrap.sh"))
            .arg("fixture@metal")
            .args(args);
        self.environment(&mut command);
        command
    }
    pub fn sync(&self, source: &Path) -> Output {
        self.bootstrap(source, &["--sync-only"]).output().expect("sync selected source")
    }
    pub fn bindings(&self) -> Vec<PathBuf> {
        snapshot(&self.remote)
            .keys()
            .filter(|path| {
                path.file_name().is_some_and(|name| name == ".overdrive-metal-checkout.v1")
            })
            .map(|path| self.remote.join(path))
            .collect()
    }
    pub fn checkout(&self) -> PathBuf {
        let bindings = self.bindings();
        assert_eq!(bindings.len(), 1, "successful sync must bind exactly its canonical checkout");
        bindings[0].parent().expect("binding parent").to_owned()
    }
    pub fn collect(&self, run_id: &str) -> Command {
        let mut command = Command::new("timeout");
        command
            .args(["--kill-after=2s", "5s", "bash"])
            .arg(workspace_file("infra/metal/lease-holder.sh"))
            .args(["--collect", run_id])
            .arg(&self.runs);
        self.environment(&mut command);
        command
    }
    pub fn retained(
        &self,
        id: &str,
        version: u8,
        terminal: Option<(&str, &str)>,
        receipt: Option<&str>,
    ) -> PathBuf {
        let directory = self.runs.join(id);
        fs::create_dir_all(&directory).expect("retained fixture");
        let metadata = format!(
            "format=overdrive-metal-run-v{version}\nrun_id={id}\ntoken={id}\nunit=overdrive-metal-run-{id}.service\ncreated_at={NOW}\nscenario_b64={}\nworkspace_b64={}\ncommit={COMMIT}\nsource_digest={DIGEST}\nremote_dir_b64={}\ntarget_user_b64={}\nexecution_mode=privileged\n{}",
            b64(b"metal isolation"),
            b64(b"/fixture/source"),
            b64(self.source.as_os_str().as_encoded_bytes()),
            b64(b"root"),
            if version == 2 { "execution_class=ordinary\n" } else { "" }
        );
        fs::write(directory.join(format!("metadata.v{version}")), metadata)
            .expect("metadata fixture");
        fs::write(
            directory.join("phase.v1"),
            format!(
                "format=overdrive-metal-phase-v1\nrun_id={id}\nphase=running\nupdated_at={NOW}\n"
            ),
        )
        .expect("phase fixture");
        if let Some((kind, value)) = terminal {
            let body = if kind == "command" {
                format!("exit_code={value}\ncause=command_returned\n")
            } else {
                format!("error={value}\n")
            };
            fs::write(directory.join("terminal.v1"), format!("format=overdrive-metal-terminal-v1\nrun_id={id}\nkind={kind}\n{body}finished_at={NOW}\n")).expect("terminal fixture");
        }
        fs::write(directory.join("output.log"), "retained selected-command output\n")
            .expect("retained log");
        fs::write(directory.join("output.log.partial"), "retained selected-command output\n")
            .expect("partial fixture");
        if let Some(kind) = receipt {
            let body = if kind == "complete" {
                "kind=complete\n".into()
            } else {
                format!("kind=runner\nerror=cleanup_failed\nroot={kind}\n")
            };
            fs::write(
                directory.join("completion.v1"),
                format!(
                    "format=overdrive-metal-completion-v1\nrun_id={id}\n{body}finished_at={NOW}\n"
                ),
            )
            .expect("receipt fixture");
        }
        directory
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let id = self.context_id();
        for root in [
            PathBuf::from("/srv/vm/metal").join(&id),
            PathBuf::from("/run/ovd-gk").join(&id),
            self.source.join("target/metal-run-tmp").join(&id),
        ] {
            if let Ok(metadata) = fs::symlink_metadata(&root) {
                if metadata.file_type().is_symlink() {
                    let _ = fs::remove_file(root);
                } else if metadata.is_dir() {
                    let _ = fs::remove_dir_all(root);
                }
            }
        }
    }
}

pub struct Durable {
    child: Child,
    pub id: String,
    pub source: PathBuf,
    runs: PathBuf,
    pub directory: PathBuf,
    output: PathBuf,
}
impl Durable {
    pub fn spawn(fixture: &Fixture, id: &str, tree: &str, class: &str, body: &str) -> Self {
        Self::spawn_with_timeout(fixture, id, tree, class, body, "1")
    }
    pub fn spawn_with_timeout(
        fixture: &Fixture,
        id: &str,
        tree: &str,
        class: &str,
        body: &str,
        timeout: &str,
    ) -> Self {
        use sha2::{Digest as _, Sha256};
        let actual_id = format!(
            "{:x}",
            Sha256::digest(
                [fixture.temp.path().as_os_str().as_encoded_bytes(), id.as_bytes()].concat()
            )
        )[..24]
            .to_owned();
        let id = actual_id.as_str();
        let remote = fixture.remote.join(tree);
        if !remote.exists() {
            fs::create_dir_all(&remote).expect("bound controller-input checkout");
            fs::copy(fixture.source.join("source-marker"), remote.join("source-marker"))
                .expect("controller source input");
            let mut identity = fs::canonicalize(&fixture.source)
                .expect("physical source")
                .as_os_str()
                .as_encoded_bytes()
                .to_vec();
            identity.push(0);
            identity.extend(git_text(&fixture.source, &["symbolic-ref", "HEAD"]).bytes());
            let key = format!(
                "{:x}",
                Sha256::digest(
                    [b"overdrive-metal-checkout-v1\0branch\0".as_slice(), &identity].concat()
                )
            );
            fs::write(remote.join(".overdrive-metal-checkout.v1"), format!("format=overdrive-metal-checkout-v1\nkind=branch\ncheckout_key={key}\nidentity_b64={}\n", b64(&identity))).expect("valid binding is a private owner-port input, not its tested output");
            fs::write(
                remote.join(".overdrive-metal-source"),
                format!(
                    "commit={COMMIT}\nworkspace={}\nsource_digest={DIGEST}\n",
                    fixture.source.display()
                ),
            )
            .expect("controller source-marker input");
        }
        let command_string = format!("cd '{}' && {body}", remote.display());
        let output = fixture.temp.path().join(format!("{id}.controller"));
        let mut command = Command::new("setsid");
        command
            .args(["timeout", "--kill-after=1s", "12s", "bash"])
            .arg(workspace_file("infra/metal/lease-holder.sh"))
            .arg("--durable")
            .arg(fixture.temp.path().join("maintenance.lock"))
            .arg(fixture.temp.path().join("maintenance.owner"))
            .args([timeout, id, "metal-isolation"])
            .arg(&fixture.source)
            .args([COMMIT, DIGEST])
            .arg(&remote)
            .args(["root", "privileged"])
            .arg(&fixture.remote)
            .arg(fixture.path())
            .arg(command_string)
            .arg(&fixture.runs)
            .arg(fixture.temp.path().join(format!("{tree}.lock")))
            .arg(fixture.temp.path().join(format!("{tree}.owner")))
            .arg(class)
            .stdin(Stdio::piped())
            .stdout(fs::File::create(&output).expect("controller output"))
            .stderr(Stdio::from(
                fs::File::create(fixture.temp.path().join(format!("{id}.stderr")))
                    .expect("controller stderr"),
            ));
        fixture.environment(&mut command);
        let child = command.spawn().expect("launch actual durable owner");
        Self {
            child,
            id: id.to_owned(),
            source: remote,
            runs: fixture.runs.clone(),
            directory: fixture.runs.join(id),
            output,
        }
    }
    pub fn wait_for(&mut self, path: &Path, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if path.is_file() && text(path).contains(needle) {
                return;
            }
            if let Some(status) = self.child.try_wait().expect("durable state") {
                panic!(
                    "durable owner ended {status} before {needle}; controller={} stderr={}",
                    text(&self.output),
                    text(&self.output.with_extension("stderr"))
                );
            }
            assert!(
                Instant::now() < deadline,
                "owner never reached {needle}; actual controller: {}",
                text(&self.output)
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn preparing(&mut self) {
        self.wait_for(&self.directory.join("phase.v1"), "phase=preparing");
    }
    pub fn send(&mut self, line: &str) {
        writeln!(self.child.stdin.as_mut().expect("control pipe"), "{line}").expect("send control");
    }
    pub fn disconnect(&mut self) {
        drop(self.child.stdin.take());
    }
    pub fn terminal(&mut self) {
        self.wait_for(&self.directory.join("terminal.v1"), "kind=");
    }
    pub fn is_live(&mut self) -> bool {
        self.child.try_wait().expect("owner liveness").is_none()
    }
}
impl Drop for Durable {
    fn drop(&mut self) {
        let _ =
            Command::new("kill").args(["-TERM", "--", &format!("-{}", self.child.id())]).status();
        let _ = self.child.wait();
        let result = Command::new("bash")
            .arg(workspace_file("infra/metal/lease-holder.sh"))
            .args(["--finalize", &self.id])
            .arg(&self.runs)
            .output();
        if let Ok(result) = result {
            let mut evidence = result.stdout;
            evidence.extend(result.stderr);
            evidence.extend(format!("\ncleanup_status={:?}\n", result.status.code()).bytes());
            let _ = fs::write(self.output.with_extension("fixture-cleanup"), evidence);
        }
    }
}
