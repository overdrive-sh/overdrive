# RCA — CLI cannot reach the control plane it just started

## Problem statement

With `overdrive serve` running and logging
`endpoint=https://127.0.0.1:7001/`, a sibling `overdrive job submit`
terminates with `CliError::Transport`:

```
Error: could not reach the control plane at https://127.0.0.1:7001/.
Cause: could not connect to server.
```

Both sides agree on the endpoint string. The server process is alive.
TCP to `127.0.0.1:7001` succeeds against the live listener. The CLI
still reports a transport failure.

## Diagnostic summary

The endpoint agreement is a red herring. The CLI's
`CliError::Transport` "could not connect to server" is the category
`stringify_reqwest_error` assigns when `reqwest::Error::is_connect()`
returns true — **which includes TLS handshake failures that occur
during the connect phase**, not just TCP `ECONNREFUSED`. The
observable symptom is therefore consistent with *either* a dead
listener or a bad TLS handshake, and the error text cannot distinguish
the two.

The listener is alive. The TLS handshake is the failure.

`overdrive serve` mints a fresh ephemeral CA on every invocation
(ADR-0010 §R1, `mint_ephemeral_ca`) and writes the new trust triple
to `<config.data_dir>/.overdrive/config`. `overdrive job submit` reads
from `default_operator_config_path()` — `$HOME/.overdrive/config`.
Those are two different files on a default invocation, and the one
the CLI reads was not written by the `serve` process in the other
terminal.

The CLI reqwest client is therefore pinning a CA that does not sign
the running server's leaf cert. rustls rejects the handshake. reqwest
reports `is_connect() == true`. The CLI renders "could not connect to
server".

## Evidence per WHY level

### WHY 1 (Symptom) — observable behaviour

**WHY 1A.** `CliError::Transport` is emitted; the server is running
and TCP-reachable.

Evidence — observed output (reproduction transcript):
```
Error: could not reach the control plane at https://127.0.0.1:7001/.
Cause: could not connect to server.
```

Evidence — error classifier that produces this text:
`crates/overdrive-cli/src/http_client.rs:307-326`

```rust
fn stringify_reqwest_error(err: &reqwest::Error) -> String {
    let category = if err.is_timeout() {
        "request timed out"
    } else if err.is_connect() {
        "could not connect to server"   // ← matched here
    } else if err.is_decode() {
        ...
```

The cause token "could not connect to server" is emitted exclusively
from the `err.is_connect()` branch. reqwest's `is_connect()` is true
for *any* error that surfaced from the connector, which includes
rustls handshake failures — it does not imply a TCP-level connection
failure.

### WHY 2 (Context) — why the TLS handshake fails

**WHY 2A.** The CA the CLI client pins as its sole root of trust did
not sign the currently-running server's leaf certificate.

Evidence — client pins exactly the CA from the loaded trust triple:
`crates/overdrive-cli/src/http_client.rs:257-289`

```rust
fn build_reqwest_client(triple: &TrustTriple) -> Result<reqwest::Client, String> {
    let ca = reqwest::Certificate::from_pem(triple.ca_cert_pem())
        .map_err(...)?;
    ...
    reqwest::Client::builder()
        .add_root_certificate(ca)
        .identity(identity)
        .https_only(true)
        .use_rustls_tls()
        ...
```

There is no system-trust fallback. The CA in the loaded trust triple
is the only root the client accepts.

Evidence — server mints a *fresh* CA on every `run_server_with_obs`
invocation: `crates/overdrive-control-plane/src/lib.rs:175`

```rust
let material = tls_bootstrap::mint_ephemeral_ca()?;
```

Evidence — `mint_ephemeral_ca` is deliberately ephemeral
(ADR-0010 §R1): `crates/overdrive-control-plane/src/tls_bootstrap.rs:90-110`

```rust
/// Mint the ephemeral CA + server leaf + client leaf.
///
/// Every call generates a fresh CA keypair — the key material never
/// leaves memory, and the function takes no configuration input by
/// design: there is no prompt and no file to read. Successive calls
/// produce distinct material.
```

So after `overdrive serve`, the running server is authenticating with
a leaf signed by a CA that was only minted at startup. Unless the CLI
is reading a trust triple *from this exact serve invocation*, the CA
it pins is stale.

### WHY 3 (System) — why the CLI is reading stale trust material

**WHY 3A.** The CLI reads from `$HOME/.overdrive/config`; `serve`
writes to `<data_dir>/.overdrive/config` — a different path on a
default invocation.

Evidence — CLI read path:
`crates/overdrive-cli/src/main.rs:61-64, 142-144`

```rust
Command::Job(JobCommand::Submit { spec }) => {
    let config_path = default_config_path();
    ...
}

fn default_config_path() -> std::path::PathBuf {
    overdrive_cli::commands::cluster::default_operator_config_path()
}
```

`crates/overdrive-cli/src/commands/cluster.rs:192-197`

```rust
pub fn default_operator_config_path() -> PathBuf {
    let base = std::env::var_os("OVERDRIVE_CONFIG_DIR")
        .or_else(|| std::env::var_os("HOME"))
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    base.join(".overdrive").join("config")
}
```

On the user's machine: `$HOME/.overdrive/config`.

Evidence — `serve` write path:
`crates/overdrive-cli/src/main.rs:99-106, 150-158`

```rust
Command::Serve { bind, data_dir } => {
    let bind_addr = bind.parse()...;
    let data_dir = data_dir.unwrap_or_else(default_data_dir);
    let args = ServeArgs { bind: bind_addr, data_dir };
    let handle = overdrive_cli::commands::serve::run(args).await?;
    ...
}

fn default_data_dir() -> std::path::PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return std::path::PathBuf::from(xdg).join("overdrive");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return std::path::PathBuf::from(home).join(".local/share/overdrive");
    }
    std::path::PathBuf::from("./overdrive")
}
```

On the user's machine: `$HOME/.local/share/overdrive`.

Evidence — `run_server_with_obs` writes the trust triple under the
data dir (NOT under the operator config dir):
`crates/overdrive-control-plane/src/lib.rs:222-230`

```rust
// Write the trust triple using the RESOLVED listener address so
// clients (tests, the CLI) load a config whose `endpoint` names
// the actual bound port. ...
let bound = std_listener.local_addr().map_err(...)?;
let endpoint = format!("https://{bound}");
tls_bootstrap::write_trust_triple(&config.data_dir, &endpoint, &material)?;
```

Evidence — `write_trust_triple` unconditionally appends
`.overdrive/config` to its `config_dir` argument:
`crates/overdrive-control-plane/src/tls_bootstrap.rs:291-301`

```rust
pub fn write_trust_triple(
    config_dir: &Path,
    endpoint: &str,
    material: &CaMaterial,
) -> Result<(), ControlPlaneError> {
    let overdrive_dir = config_dir.join(".overdrive");
    std::fs::create_dir_all(&overdrive_dir).map_err(...)?;
    let config_path = overdrive_dir.join("config");
    ...
}
```

So on a default `overdrive serve` invocation the trust triple lands
at `$HOME/.local/share/overdrive/.overdrive/config` — not at
`$HOME/.overdrive/config` where the CLI reads.

Evidence — the `~/.overdrive/config` the CLI *does* read was written
earlier by `cluster init`: `crates/overdrive-cli/src/commands/cluster.rs:76-93`

```rust
let config_dir = resolve_config_dir(args.config_dir)?;
...
let endpoint_str = "https://127.0.0.1:7001";
...
let material = mint_ephemeral_ca().map_err(...)?;
write_trust_triple(&config_dir, endpoint_str, &material)
    .map_err(...)?;
```

`cluster init` and `serve` each mint their own CA. The endpoint
strings match by default. The CA material does not.

### WHY 4 (Design) — why the write path and read path compute
different paths

**WHY 4A.** `serve::default_data_dir` and `job::default_config_path`
resolve from different env vars (`$XDG_DATA_HOME` vs
`$OVERDRIVE_CONFIG_DIR`), fall back to different `$HOME` suffixes
(`.local/share/overdrive` vs `.overdrive`), and neither is the
canonical operator config path. The two are held in separate helper
functions in `main.rs` that nothing structurally couples; there is no
shared invariant enforcing "the place `serve` writes the trust triple
is the place `job submit` reads it".

Evidence — the two defaults live adjacently in `main.rs` with no
cross-reference: `crates/overdrive-cli/src/main.rs:128-158`

**WHY 4B.** `write_trust_triple`'s signature takes a `config_dir`
that means "the parent under which to create `.overdrive/config`" —
which is correct for the CLI's `cluster init` path but a category
confusion when called from `run_server_with_obs` with `config.data_dir`.
The data dir is a storage root (redb, per-primitive libSQL); the
operator config path is an identity artefact. Overloading one
directory argument to carry both roles is the structural defect.

Evidence — `ServerConfig` and the `run_server_with_obs` call site:
`crates/overdrive-control-plane/src/lib.rs:75-85, 222-230`

```rust
/// Configuration for the Phase 1 control-plane server. ...
pub struct ServerConfig {
    pub bind: SocketAddr,
    /// Data directory — parent of the redb file, per-primitive libSQL
    /// files, and the trust triple config file.
    pub data_dir: PathBuf,
}
```

The doc comment admits the overload: "parent of the redb file, ... AND
the trust triple config file". Under an XDG-respecting CLI, those two
roles map to different directories.

**WHY 4C.** Every integration test passes the same `TempDir` as both
`data_dir` and `config_path` base, so the overload is invisible:
`crates/overdrive-cli/tests/integration/cluster_init_serve.rs:133-154`

```rust
let tmp = TempDir::new().expect("tempdir");
...
let args = ServeArgs { bind, data_dir: tmp.path().to_path_buf() };
let handle: ServeHandle = overdrive_cli::commands::serve::run(args).await...;
...
let config_path = tmp.path().join(".overdrive").join("config");
let client = build_client(&config_path);
```

The test uses exactly the path the server wrote. Production uses a
different path, and no test exercises that production path.

### WHY 5 (Root cause)

**ROOT CAUSE A — Structural.** The CLI has two helper functions for
computing "where operator trust material lives", and they disagree on
a default invocation. `default_operator_config_path()` returns
`$HOME/.overdrive/config` (canonical, per whitepaper §8 / ADR-0019,
and shared between read and write sites of `cluster init`).
`default_data_dir()` returns `$HOME/.local/share/overdrive`, and
`run_server_with_obs` blindly appends `.overdrive/config` under
*that* directory to write the trust triple — so `serve` deposits the
triple at `$HOME/.local/share/overdrive/.overdrive/config` instead
of the canonical `$HOME/.overdrive/config`. There is no structural
invariant that ties the server's trust-triple write target to the
operator CLI's read target.

**ROOT CAUSE B — Definitional.** `ServerConfig::data_dir` is
overloaded: it carries two unrelated roles (storage root for redb +
libSQL per ADR-0013 §5 *and* operator-config root for the trust
triple). When only one directory argument is supplied, the two roles
must collapse to the same path. `cluster init` collapses them at the
CLI boundary by passing the operator config dir. `serve` does not —
it passes the data dir and writes the trust triple into it, which is
the wrong lineage. The same overload is why every integration test
works — the test can pass one `TempDir` because the two roles share
a single path in-test.

**ROOT CAUSE C — Test coverage.** No integration test exercises the
end-to-end `serve` → `job submit` flow with the real production path
defaults. Every existing test either (a) calls `cluster init` with
an explicit `TempDir` and asserts nothing about `serve`, or
(b) calls `serve::run` with a `TempDir` data dir and reads from
`tmp.path().join(".overdrive/config")`. A test that mirrors the
production binary — default `serve` defaults, default
`job submit` defaults, `$HOME` pointing at a tempdir — would fail
today. The recent HOME-fallback regression tests in
`cluster_init_serve.rs` pin the `init`-write / `job`-read invariant,
but the `serve`-write / `job`-read invariant is untested.

## Cross-validation

**Backwards chain A.** If `serve` writes to
`$HOME/.local/share/overdrive/.overdrive/config` and the CLI reads
from `$HOME/.overdrive/config`, would the CLI load *some* trust
triple? Only if one exists at the read path. The reproduction
transcript shows the user's `~/.overdrive/config` is populated
(`current-context = "local"` present), which means it was written by
a prior `cluster init`. The CA there has no relationship to the CA
the currently-running `serve` process minted. Handshake fails →
reqwest `is_connect()` → "could not connect to server". Chain closes.

**Backwards chain B.** If the two directories accidentally coincided
(e.g. the user set `OVERDRIVE_CONFIG_DIR=$HOME/.local/share/overdrive`,
or the user ran `overdrive serve --data-dir ~/` so the `.overdrive`
suffix collides with `$HOME/.overdrive`), would the bug disappear?
Yes — both paths would resolve to the same file and the CA pinned by
the CLI would be the one the server just minted. That exact pattern
is what every test does implicitly. Chain closes.

**No contradictions between A, B, and C.** A and B describe the same
structural defect from two sides; C explains why the defect was not
caught. All three are consistent and together explain the observed
symptom completely.

**All symptoms explained.** The transcript's three surface
observations — log endpoint matches config endpoint, server is still
running, CLI reports transport failure — are all consistent with
"TLS handshake failure due to CA mismatch" and none of them require
the other hypotheses (IPv4/IPv6, silent background-task panic, stale
log source) to be true.

## Hypotheses considered and rejected

| Hypothesis | Why rejected |
|---|---|
| Server logs a hardcoded address but actually binds elsewhere | `handle.endpoint()` flows from `local_addr()` on the `axum_server::Handle`, which is populated by the listening notification — the log line is the resolved bind address. `main.rs:106` reads directly from `handle.endpoint()`. |
| IPv6 / IPv4 split (server binds `::1`, client resolves `127.0.0.1` or vice versa) | The server leaf carries SANs for both `127.0.0.1` and `::1` (`tls_bootstrap.rs:155-157`); reqwest resolves `127.0.0.1` to IPv4 and the listener accepts IPv4. Both sides would succeed if the trust material matched. |
| Server's background task panics silently after logging | `axum_server::from_tcp_rustls` is invoked only *after* `TcpListener::bind` returns successfully; the bind is synchronous and surfaces errors to `run_server` before logging. The user explicitly states the server is still running. |
| Handshake failure is the cause — but for a different reason (wrong key type, cert expiry) | Cert expiry is not implicated: a fresh mint has a year-long default TTL. Key type is P-256 on both ends. The only CA/server/client mismatch that `mint_ephemeral_ca` invariants permit is two separate mints — which is exactly what this bug is. |
| Cluster init wrote the wrong `endpoint` value (the `fix-overdrive-config-path-doubled` regression) | Transcript shows `~/.overdrive/config` exists and parses (the CLI reaches `CliError::Transport`, not `CliError::ConfigLoad`), and the endpoint field matches `https://127.0.0.1:7001` — the path-doubling bug is not present here. |

## Contributing factors (adjacent smells)

1. **`ServerConfig::data_dir` is overloaded.** It carries both
   "storage root for redb + libSQL" and "root under which the
   operator trust triple lives". These are different concepts per
   ADR-0013 §5 vs whitepaper §8. Callers that treat them as one
   directory work only when the two coincide.
2. **`stringify_reqwest_error` classifies TLS handshake failures as
   "could not connect to server".** The category is technically
   correct for reqwest's internal model but actively misleading for
   operators — "connection refused", "handshake failed", and "bad
   certificate" are three distinct diagnostic categories that should
   not collapse to one message. See Prevention Strategy below.
3. **Every integration test uses one tempdir for both roles.** The
   test shape hides the production failure rather than exposing it.
4. **`run_server` / `run_server_with_obs` have no contract that the
   trust triple must be written where the operator CLI will read
   it.** The server writes the triple as a convenience; the CLI
   assumes the triple is there; nothing mechanical ties the two.

## Proposed fix (diagnose only — do NOT implement)

The single smallest change that closes the root cause:

**Stop writing the trust triple from the server's boot path. Make
`overdrive serve` either (a) require an existing trust triple at the
operator config path, or (b) write to the operator config path, not
the data dir.**

Option (b) is the lower-friction fix for Phase 1. Concretely:

1. **`crates/overdrive-cli/src/main.rs:99-106`** — compute the
   operator config dir *in addition to* the data dir, and thread it
   into `ServeArgs`:
   ```rust
   Command::Serve { bind, data_dir } => {
       let bind_addr = bind.parse()...;
       let data_dir = data_dir.unwrap_or_else(default_data_dir);
       let config_dir = default_operator_config_dir();  // NEW
       let args = ServeArgs { bind: bind_addr, data_dir, config_dir };
       ...
   }
   ```
   `default_operator_config_dir()` returns the base dir — the same
   `$OVERDRIVE_CONFIG_DIR`-or-`$HOME` resolution already used by
   `default_operator_config_path`, stopping BEFORE the `.overdrive`
   suffix.
2. **`crates/overdrive-cli/src/commands/serve.rs:28-36`** — add
   `config_dir: PathBuf` to `ServeArgs`.
3. **`crates/overdrive-cli/src/commands/serve.rs:86-106`** — pass
   `config_dir` through to `ServerConfig`.
4. **`crates/overdrive-control-plane/src/lib.rs:75-85`** — add an
   `operator_config_dir: PathBuf` field to `ServerConfig` (or rename
   `data_dir` and introduce `operator_config_dir` as a distinct
   field). The data dir stays for redb + libSQL; the operator config
   dir is where the trust triple goes.
5. **`crates/overdrive-control-plane/src/lib.rs:222-230`** — write
   the trust triple to `config.operator_config_dir`, not
   `config.data_dir`. `write_trust_triple`'s contract is unchanged —
   it still appends `.overdrive/config`.
6. **`crates/overdrive-control-plane/tests/integration/*.rs`** and
   **`crates/overdrive-cli/tests/integration/*.rs`** — where tests
   currently pass one `TempDir` as `data_dir` and read from
   `tmp.path().join(".overdrive/config")`, they need two separate
   subdirectories (e.g. `tmp.path().join("data")` and
   `tmp.path().join("conf")`) so the two roles remain decoupled
   in-test. This exposes the production failure if a future refactor
   re-overloads them.

The lower-risk interim alternative — keeping `data_dir` overloaded
but making it resolve to the operator config dir by default — is
rejected. ADR-0013 §5 explicitly identifies the data dir as XDG
`data_dir()/overdrive`; forcing `serve` to use the operator config
dir as its storage root would collide with the ADR and land redb +
libSQL files under `$HOME/.overdrive/`, which is an identity
directory, not a data directory.

### Secondary fix (diagnostic quality; do NOT implement in the same
change without review)

**`crates/overdrive-cli/src/http_client.rs:307-326`** — split the
`is_connect()` arm so TLS handshake failures render as a distinct
cause from TCP connect refused. reqwest exposes the chained source
via `err.source()`; a rustls certificate error in the chain can be
recognised and rendered as "TLS handshake failed (certificate not
trusted)" with a hint to re-run `overdrive cluster init` if the CA
was re-minted. This is a diagnostic improvement, not a root-cause
fix — leave it as a follow-up so the primary fix's diff stays
minimal.

## Files affected (primary fix)

| File | Change |
|---|---|
| `crates/overdrive-cli/src/main.rs` | Thread `config_dir` into `ServeArgs`; add `default_operator_config_dir()` helper next to the existing `default_config_path` / `default_data_dir`. |
| `crates/overdrive-cli/src/commands/serve.rs` | Add `config_dir: PathBuf` field to `ServeArgs`; forward into `ServerConfig`. |
| `crates/overdrive-cli/src/commands/cluster.rs` | Expose a `default_operator_config_dir()` base-dir helper next to `default_operator_config_path()` (or extend the existing helper to return both, per the author's judgment on the cleanest shape). |
| `crates/overdrive-control-plane/src/lib.rs` | Add `operator_config_dir: PathBuf` to `ServerConfig`; rewrite `run_server_with_obs` to pass it to `write_trust_triple`. The `config.data_dir` argument stays as the redb + libSQL root. |
| `crates/overdrive-control-plane/tests/integration/{describe_round_trip,server_lifecycle,observation_empty_rows,submit_round_trip,concurrent_submit_toctou,idempotent_resubmit}.rs` | Pass separate `data_dir` and `operator_config_dir` subdirectories of the `TempDir`. Read trust triple from the new `operator_config_dir` path. |
| `crates/overdrive-cli/tests/integration/cluster_init_serve.rs` | `serve_run_binds_ephemeral_port_and_returns_serve_handle` and `probe_after_shutdown_returns_transport_error` — pass separate `data_dir` and `operator_config_dir`; read from `operator_config_dir`. |
| `crates/overdrive-cli/tests/integration/job_submit.rs` | Same decoupling as above. |
| `crates/overdrive-cli/tests/integration/http_client.rs`, `endpoint_from_config.rs`, `walking_skeleton.rs`, `cluster_and_node_commands.rs` | Same decoupling. |
| *Regression test* (new) | Step `(g)` in `cluster_init_serve.rs`: full-production-defaults test that scopes `$HOME` to a tempdir, runs `serve` + `job submit` via their default helpers, and proves the CLI can reach the server *without* any explicit path plumbing. |

## Risk assessment

**What could break**

- Any downstream consumer that constructed `ServerConfig` directly
  with only `bind` and `data_dir` — all such sites are inside this
  workspace; a breaking change to `ServerConfig` is permissible per
  the *single-cut greenfield migrations* memory. Every existing
  caller is updated in the same PR.
- Tests that read `tmp.path().join(".overdrive/config")` after
  running `serve::run` — those tests are not wrong, they are just
  out of date relative to the fix. Updating them is mechanical.
- No production operator runs the binary today (Phase 1 walking
  skeleton), so there is no operator-facing compatibility window to
  preserve. The `~/.overdrive/config` written by `cluster init`
  continues to work exactly as before — the CA mismatch only
  disappears because `serve` now rewrites the same file instead of
  writing a separate one.

**What could be made subtly worse**

- After the fix, `overdrive serve` *overwrites* whatever
  `~/.overdrive/config` already exists. This is consistent with
  `cluster init`'s ADR-0010 §R4 re-mint-unconditionally behaviour,
  but it means an operator who ran `cluster init` and then ran
  `serve` will lose the `cluster init` material. This is not a
  regression — it is the behaviour the user assumed was already
  happening, and it removes the "whose CA is loaded" ambiguity. Note
  the behaviour in `serve`'s `--help` output and in the
  `cluster_init_serve.rs` acceptance test preamble.
- Persistence: because `serve` now writes the trust triple on every
  run, restarting the server mints a new CA and invalidates any
  previously-opened CLI sessions. Phase 1's ephemeral-CA model
  already implies this; it becomes more visible after the fix. This
  belongs in a Phase-2 persisted-CA story, not in this bugfix.

**What tests the fix needs**

Four-tier stack mapping:

- **Tier 1 (DST)** — not required. The defect is not a concurrency,
  timing, or partition issue; it is a path-composition defect. DST
  would add no signal.
- **Newtype / proptest** — not required. No newtype is in play; the
  failing invariant is path equality, not a value-space property.
- **trybuild** — not required. No type-system property is changing.
- **Integration (`integration-tests` feature, per testing.md)** —
  **required**. The new regression test stands up `serve` and
  `job submit` via their default helpers, scopes `$HOME` via
  `#[serial_test::serial(env)]`, and asserts the full round-trip
  works. This test must fail against `main` and pass against the
  fix. It is the only test that would have caught this class of bug
  and must not be omitted. See sketch below.
- **`cargo xtask mutants --diff origin/main`** — the kill-rate gate
  covers the path the fix touches. Existing mutation coverage for
  `write_trust_triple` and `default_operator_config_path` is already
  in place; the new `default_operator_config_dir` helper plus the
  new `operator_config_dir` field carry their own mutation coverage
  through the round-trip test.

## Regression test shape

Place the test in
`crates/overdrive-cli/tests/integration/cluster_init_serve.rs`
alongside the existing HOME-fallback tests. Use
`#[serial_test::serial(env)]` — the test mutates `$HOME` and
`$OVERDRIVE_CONFIG_DIR` to scope them to a tempdir. Per
`.claude/rules/testing.md`, the test lives under the existing
`integration-tests` feature entrypoint; no new CI wiring is required
because the `integration` nextest binary already exists.

Sketch:

```rust
#[tokio::test]
#[serial(env)]
async fn serve_and_submit_with_production_defaults_succeeds() {
    // Scope HOME to a tempdir so default_data_dir() and
    // default_operator_config_path() both resolve under the tempdir.
    let tmp = TempDir::new().expect("tempdir");
    let _guard = EnvGuard::scoped(&[
        ("HOME", Some(tmp.path())),
        ("OVERDRIVE_CONFIG_DIR", None),
        ("XDG_DATA_HOME", None),
    ]);

    // 1. Start the server via the SAME path main.rs uses.
    let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let data_dir =
        tmp.path().join(".local/share/overdrive"); // mirrors default_data_dir
    let config_dir = tmp.path().join(".overdrive"); // mirrors default_operator_config_dir
    let handle = overdrive_cli::commands::serve::run(ServeArgs {
        bind,
        data_dir,
        config_dir, // NEW field after the fix
    })
    .await
    .expect("serve::run");

    // 2. Submit via the SAME path main.rs uses. NO explicit
    //    config_path — the handler must pick it up via
    //    default_operator_config_path().
    let config_path =
        overdrive_cli::commands::cluster::default_operator_config_path();
    assert_eq!(
        config_path,
        tmp.path().join(".overdrive").join("config"),
        "production default must land under $HOME/.overdrive/config",
    );

    // Spec loader + submit. The assertion we care about is that
    // `submit` does NOT return CliError::Transport — the endpoint is
    // reachable AND trust material matches.
    let spec_path = write_minimal_job_spec(tmp.path());
    let out = overdrive_cli::commands::job::submit(
        overdrive_cli::commands::job::SubmitArgs {
            spec: spec_path,
            config_path,
        },
    )
    .await
    .expect(
        "CLI must reach the server it just started — \
         this is exactly the bug the test pins",
    );

    assert!(
        out.endpoint.as_str().contains("127.0.0.1"),
        "submit must have reached the live server; got endpoint {}",
        out.endpoint,
    );

    handle.shutdown().await.expect("clean shutdown");
}
```

This test is load-bearing. Against current `main` it reports
`CliError::Transport` because `serve::run` writes to
`tmp.path()/.local/share/overdrive/.overdrive/config` while
`default_operator_config_path()` points at
`tmp.path()/.overdrive/config`. Against the fix it succeeds because
both sites resolve to the same file.

The `EnvGuard` helper already exists in
`cluster_init_serve.rs` for the HOME-fallback regression tests
(`development.md` lifetime-discipline section rejects the per-test
manual save-and-restore pattern in favour of an RAII guard). Reuse
it.

## Prevention strategy

Beyond the fix itself:

1. **Decouple operator-config path from data path at the type
   level.** Both `ServerConfig::data_dir` and the new
   `operator_config_dir` field should be newtypes rather than raw
   `PathBuf`s (`DataDir(PathBuf)` and `OperatorConfigDir(PathBuf)`)
   so the two cannot be swapped at a call site. This is a Phase 1
   follow-up; the immediate fix renames fields without introducing
   newtypes to keep the change minimal.
2. **Make the CLI's HTTP error classifier distinguish handshake
   failures from TCP refused** (secondary fix above). A TLS error
   that renders as "could not connect to server" actively
   misdirects the operator toward network debugging when the real
   issue is trust material. This is a diagnostic-quality
   improvement, not a root-cause fix — ship it in a follow-up PR.
3. **Add an integration-test convention:** any test that exercises
   the CLI's production-default path MUST scope `$HOME` /
   `$OVERDRIVE_CONFIG_DIR` / `$XDG_DATA_HOME` and call the handler
   with no explicit paths. Convention lives in
   `crates/overdrive-cli/CLAUDE.md` under the existing "Integration
   tests — no subprocess" section.
4. **ADR alignment.** The ADR (ADR-0013 §5 or ADR-0010, whichever
   is the structural home) should note explicitly that the operator
   config directory and the node data directory are separate
   concerns — this is implicit today and a reader can only learn it
   by reconciling `default_data_dir` against `default_operator_config_path`
   in `main.rs`.

## Effort

- Primary fix: ~2–3 hours including test updates and the new
  regression test.
- Secondary fix (error-classifier split): ~1 hour, separate PR.
- ADR clarification: ~30 minutes, separate PR, dispatched to the
  architect agent per the `feedback_delegate_to_architect` memory.
