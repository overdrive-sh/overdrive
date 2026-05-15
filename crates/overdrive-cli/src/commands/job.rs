//! `overdrive job submit`.
//!
//! Reads a TOML job spec from disk, runs `Job::from_submit` locally for
//! fast-fail validation, POSTs the typed `SubmitWorkloadRequest` to the
//! control plane, and returns a typed [`SubmitOutput`] carrying the
//! `workload_id`, derived `intent_key`, canonical `spec_digest`, idempotency
//! `outcome`, endpoint, and operator next-command hint.
//!
//! Per ADR-0020 (drop `commit_index` from Phase 1) the wire shape is
//! `{workload_id, spec_digest, outcome}` — the Raft commit-index field was
//! dropped. `spec_digest` is the lowercase-hex SHA-256 of the canonical
//! rkyv-archived `Job` bytes (ADR-0002), 64 characters; `outcome` is
//! `IdempotencyOutcome::{Inserted, Unchanged}`.
//!
//! Per ADR-0011, `Job::from_submit` is THE validating constructor. The
//! CLI runs it client-side for an immediate, operator-facing error
//! that names the offending field without a server round-trip; the
//! server runs it again on ingress for defence-in-depth (ADR-0015).
//!
//! Per `crates/overdrive-cli/CLAUDE.md` the handler is a plain
//! `async fn` that tests call directly — no subprocess, no `println!`.
//! Rendering lives in `crate::render::workload_submit_accepted`.

use std::path::PathBuf;

use bytes::BytesMut;
use futures::StreamExt as _;
use overdrive_control_plane::api::{
    IdempotencyOutcome, StopOutcome, SubmitEvent, SubmitWorkloadRequest, TerminalReason,
};
use overdrive_control_plane::streaming::JobSubmitEvent;
use overdrive_core::TransitionReason;
use overdrive_core::aggregate::{
    AggregateError, DriverInput, ExecInput as LegacyExecInput, IntentKey, Job, JobSpec,
    JobSpecInput, ResourcesInput as LegacyResourcesInput, WorkloadSpecInput,
};
use overdrive_core::api::submit::SubmitSpecInput;
use overdrive_core::id::WorkloadId;
use url::Url;

use crate::http_client::{ApiClient, CliError};

// ---------------------------------------------------------------------------
// IsTerminal auto-detach — Slice 03 step 03-02.
// ---------------------------------------------------------------------------
//
// architecture.md §6 + DESIGN [D5]: the CLI's lane decision is
//
//   stream = !args.detach && std::io::IsTerminal::is_terminal(&std::io::stdout())
//
// `stream == true` engages the NDJSON streaming consumer
// (`Accept: application/x-ndjson`); `stream == false` engages the
// JSON-ack lane (`Accept: application/json`). Reference class:
// `docker run`, `nomad job run`, every Unix-tradition CLI tool.
//
// The `IsTerminal` probe is hidden behind a small trait seam so the
// dispatch decision is testable in-process — `crates/overdrive-cli/CLAUDE.md`
// forbids `Command::spawn` in tests, and reading the real `stdout`
// inside a test process is non-deterministic (cargo nextest captures
// output by default, returning `false` from `IsTerminal::is_terminal`).
// Production wires `RealStdoutTerminal` (which calls the std lib);
// tests wire fakes returning a fixed boolean.

/// Probe for whether the binary's stdout is currently attached to a TTY.
///
/// Production wires [`RealStdoutTerminal`]; Tier 1 acceptance tests
/// wire fakes with deterministic return values to drive the truth
/// table at `tests/acceptance/submit_pipe_autodetect.rs`.
///
/// The trait is `Send + Sync` so an instance can be shared across
/// `tokio` task boundaries inside `main.rs`'s clap dispatch — even
/// though Phase 1's `run` is single-threaded once the runtime is up,
/// keeping the bound makes future call-site refactors painless.
pub trait StdoutTerminalProbe: Send + Sync {
    /// Returns `true` iff the binary's stdout is attached to a TTY.
    /// Implementations MUST be deterministic for the duration of a
    /// single CLI invocation — flipping mid-run would yield a different
    /// dispatch decision than the auto-detach truth table promises.
    fn is_terminal(&self) -> bool;
}

/// Production [`StdoutTerminalProbe`] — defers to
/// [`std::io::IsTerminal::is_terminal`] on `std::io::stdout()`.
///
/// Wired into `main.rs`'s clap dispatch as the only production source
/// of the `IsTerminal` bit. Construction is zero-allocation; the type is
/// a unit struct.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealStdoutTerminal;

impl StdoutTerminalProbe for RealStdoutTerminal {
    fn is_terminal(&self) -> bool {
        std::io::IsTerminal::is_terminal(&std::io::stdout())
    }
}

/// Compute the auto-detach lane decision from the `--detach` flag and
/// the `IsTerminal` probe result.
///
/// Truth table (architecture.md §6, DESIGN [D5]):
///
/// | `detach` | `is_terminal` | result | lane                     |
/// |----------|---------------|--------|--------------------------|
/// | `true`   | any           | `false`| JSON-ack (Detached)      |
/// | `false`  | `true`        | `true` | NDJSON streaming         |
/// | `false`  | `false`       | `false`| JSON-ack (Detached)      |
///
/// Returns `true` iff the streaming-NDJSON consumer should be engaged.
///
/// This is the SSOT for the dispatch decision — `main.rs` calls
/// `should_stream(detach, probe.is_terminal())` and branches between
/// `submit_streaming` (true) and `submit` (false). Acceptance tests
/// at `tests/acceptance/submit_pipe_autodetect.rs` exercise this
/// pure function directly; the wire-level Accept-header pinning is
/// covered by the existing JSON-ack and streaming integration suites.
#[must_use]
pub const fn should_stream(detach: bool, is_terminal: bool) -> bool {
    !detach && is_terminal
}

/// Arguments to [`submit`].
///
/// `spec` is the path to a TOML file containing a `JobSpecInput`-shaped
/// document; `config_path` locates the operator trust triple, which is
/// the sole source of the control-plane endpoint per whitepaper §8.
#[derive(Debug, Clone)]
pub struct SubmitArgs {
    /// Path to the TOML job spec on disk.
    pub spec: PathBuf,
    /// Path to the Talos-shape trust triple on disk. The endpoint
    /// recorded in the triple is where the POST is issued.
    pub config_path: PathBuf,
}

/// Typed output of a successful `job submit`.
///
/// Carries the server's assigned `workload_id`, the derived `intent_key`
/// (`jobs/<id>`), the canonical `spec_digest`, the idempotency
/// `outcome`, the endpoint actually `POST`ed to, and the operator
/// next-command hint.
///
/// Per ADR-0020 the Raft `commit_index` field is dropped — it was an
/// in-memory `u64` and never a substitute for an authoritative
/// observability surface.
///
/// Handlers never render output themselves; the binary wrapper passes
/// this value to [`crate::render::workload_submit_accepted`].
#[derive(Debug, Clone)]
pub struct SubmitOutput {
    /// Job ID echoed by the server — matches the `id` field of the
    /// input spec after validation.
    pub workload_id: String,
    /// Derived intent-store key — `jobs/<workload_id>` per ADR-0011 §`IntentKey`.
    pub intent_key: String,
    /// Lowercase-hex SHA-256 of the canonical rkyv-archived `Job`
    /// bytes (ADR-0002, development.md §Hashing); 64 characters.
    /// Stable across byte-identical resubmissions.
    pub spec_digest: String,
    /// Idempotency outcome echoed by the control plane. `Inserted` on
    /// fresh submission, `Unchanged` on a byte-identical resubmission
    /// at the same intent key per ADR-0015 §4 (amended by ADR-0020).
    pub outcome: IdempotencyOutcome,
    /// Endpoint the POST was issued to, echoed for operator clarity.
    pub endpoint: Url,
    /// Next-command hint the operator can run to inspect allocation
    /// status — `overdrive alloc status --job <workload_id>`.
    pub next_command: String,
}

/// Submit a job spec to the control plane.
///
/// # Errors
///
/// * [`CliError::InvalidSpec`] — the TOML file is unreadable,
///   malformed, or fails `Job::from_submit` (zero replicas, zero memory,
///   unparseable ID). Fires BEFORE any HTTP call.
/// * [`CliError::ConfigLoad`] — the trust triple cannot be loaded.
/// * [`CliError::Transport`] — the control plane is unreachable.
/// * [`CliError::HttpStatus`] — the server returned 4xx / 5xx.
/// * [`CliError::BodyDecode`] — the 2xx response body failed to parse.
pub async fn submit(args: SubmitArgs) -> Result<SubmitOutput, CliError> {
    // 1. Read TOML from disk. Missing / unreadable files map to
    //    InvalidSpec with field="spec" so the operator can fix the path.
    let toml_str = std::fs::read_to_string(&args.spec).map_err(|e| CliError::InvalidSpec {
        field: "spec".to_string(),
        message: format!("failed to read `{}`: {e}", args.spec.display()),
    })?;

    // 2. Try the kind-discriminated parser first (same detection as
    //    `submit_streaming`). If the TOML carries a `[job]` section,
    //    translate to the wire shape via `JobSpecInput`. Flat TOMLs
    //    fall through to the legacy `JobSpecInput` parser. Per
    //    ADR-0051 the wire-side discriminator now lives inside
    //    `SubmitSpecInput`'s `kind` tag; the outer
    //    `SubmitWorkloadRequest.workload_kind` field has been deleted.
    let spec_input: JobSpecInput =
        if let Ok(WorkloadSpecInput::Job(job_spec)) = WorkloadSpecInput::from_toml_str(&toml_str) {
            JobSpecInput {
                id: job_spec.id,
                replicas: 1,
                driver: DriverInput::Exec(LegacyExecInput {
                    command: job_spec.exec.command,
                    args: job_spec.exec.args,
                }),
                resources: LegacyResourcesInput {
                    cpu_milli: job_spec.resources.cpu_milli,
                    memory_bytes: job_spec.resources.memory_bytes,
                },
            }
        } else {
            toml::from_str(&toml_str).map_err(|e| CliError::InvalidSpec {
                field: "toml".to_string(),
                message: format!("failed to parse TOML: {e}"),
            })?
        };

    // 3. Client-side validation via the shared ADR-0011 constructor.
    //    Fast-fail BEFORE any HTTP call — operators see the offending
    //    field without a round-trip.
    let _validated: Job = Job::from_submit(spec_input.clone()).map_err(aggregate_to_cli_error)?;

    // 4. Build the typed API client and POST. The endpoint is the one
    //    recorded in the trust triple — the operator config is the
    //    sole source.
    let client = ApiClient::from_config(&args.config_path)?;
    let endpoint = client.base_url().clone();
    let resp = client
        .submit_workload(SubmitWorkloadRequest { spec: SubmitSpecInput::Job(spec_input) })
        .await?;

    // 5. Compose the typed output. Intent key is derived via the
    //    shared `IntentKey::for_workload` helper (ADR-0050 OQ-5 single-
    //    cut migration: `jobs/<id>` → `workloads/<id>`) — no drift-
    //    prone second `workloads/` literal in this crate.
    let workload_id = parse_response_job_id(&resp.workload_id)?;
    let intent_key = IntentKey::for_workload(&workload_id).as_str().to_string();
    let next_command = format!("overdrive alloc status --job {}", resp.workload_id);
    Ok(SubmitOutput {
        workload_id: resp.workload_id,
        intent_key,
        spec_digest: resp.spec_digest,
        outcome: resp.outcome,
        endpoint,
        next_command,
    })
}

/// Parse a `workload_id` string echoed back in a successful 2xx control-plane
/// response into a typed [`WorkloadId`].
///
/// On `WorkloadId::new` failure, the call site at [`submit`] is *post-HTTP*:
/// the server returned a 200 OK whose `workload_id` field cannot be parsed by
/// the same validating constructor the spec went through. Per the
/// rustdoc on [`CliError::InvalidSpec`] (client-side spec validation
/// BEFORE any HTTP call) and [`CliError::BodyDecode`] (a successful 2xx
/// response whose body failed to deserialise into the expected typed
/// shape — server-side contract violation), this is a `BodyDecode`
/// shape, not an `InvalidSpec` shape.
pub fn parse_response_job_id(raw: &str) -> Result<WorkloadId, CliError> {
    WorkloadId::new(raw).map_err(|e| CliError::BodyDecode {
        cause: format!("server returned invalid workload_id `{raw}`: {e}"),
    })
}

// ---------------------------------------------------------------------------
// `overdrive job stop <id>` — Step 02-04 / Slice 3B (US-03 stop scope).
// ---------------------------------------------------------------------------

/// Arguments to [`stop`].
#[derive(Debug, Clone)]
pub struct StopArgs {
    /// Canonical `WorkloadId` to stop. Validated client-side via
    /// `WorkloadId::new` before any HTTP call so operators see the
    /// offending byte without a round-trip.
    pub id: String,
    /// Path to the trust triple. Same conventions as [`SubmitArgs`].
    pub config_path: PathBuf,
}

/// Typed output of `overdrive job stop`. Carries the server's echoed
/// `workload_id`, the `outcome` (`Stopped` vs `AlreadyStopped`), the endpoint
/// the POST was issued to, and the operator's next-step hint.
#[derive(Debug, Clone)]
pub struct StopOutput {
    pub workload_id: String,
    pub outcome: StopOutcome,
    pub endpoint: Url,
}

/// Stop a previously-submitted job by writing the stop intent.
///
/// Per ADR-0027: returns 200 OK with `outcome = Stopped` on first
/// stop and `AlreadyStopped` on idempotent re-stop. Returns 404 if
/// the job was never submitted.
///
/// # Errors
///
/// * [`CliError::InvalidSpec`] — `id` does not parse as a canonical `WorkloadId`.
/// * [`CliError::ConfigLoad`] — trust triple unloadable.
/// * [`CliError::Transport`] — control plane unreachable.
/// * [`CliError::HttpStatus`] — server returned non-2xx (404 unknown).
/// * [`CliError::BodyDecode`] — 2xx body decode failed.
pub async fn stop(args: StopArgs) -> Result<StopOutput, CliError> {
    // Client-side validation — fail fast on malformed ids.
    let _ = WorkloadId::new(&args.id)
        .map_err(|e| CliError::InvalidSpec { field: "id".to_string(), message: e.to_string() })?;

    let client = ApiClient::from_config(&args.config_path)?;
    let endpoint = client.base_url().clone();
    let resp = client.stop_workload(&args.id).await?;

    Ok(StopOutput { workload_id: resp.workload_id, outcome: resp.outcome, endpoint })
}

// ---------------------------------------------------------------------------
// `overdrive job submit` — streaming NDJSON consumer (Slice 02 step 02-04).
// ---------------------------------------------------------------------------

/// Typed output of a successful streaming `job submit`.
///
/// Per slice 02 step 02-04 acceptance criteria, `submit_streaming`
/// consumes the `application/x-ndjson` stream until a terminal
/// `ConvergedRunning` or `ConvergedFailed` event arrives. The handler
/// returns this typed shape carrying:
///
///  * `Accepted`-event-derived fields (`workload_id`, `intent_key`,
///    `spec_digest`, `outcome`) — same shape as the one-shot ack lane
///    so existing renderer/tests keep their assertion shapes.
///  * `exit_code` — 0 on `ConvergedRunning`, 1 on `ConvergedFailed`. The
///    binary wrapper at `main.rs` surfaces this as the process exit
///    code, satisfying ADR-0032 §9.
///  * `summary` — operator-facing rendered text written to stdout (the
///    success summary line for `Running`, the structured `Error:` block
///    for `Failed`).
///  * `terminal_reason` / `streaming_reason` / `streaming_error` —
///    typed projections of the terminal `SubmitEvent` payloads, used
///    by the S-WS-02 KPI-02 byte-equality assertions.
///
/// Pre-Accepted failures (4xx/5xx, transport errors, malformed spec)
/// short-circuit BEFORE this struct is constructed and surface as
/// `Err(CliError)` per [`crate::http_client::CliError`].
#[derive(Debug, Clone)]
pub struct SubmitStreamingOutput {
    /// Job ID echoed by the server's `Accepted` event.
    pub workload_id: String,
    /// Derived intent-store key — `jobs/<workload_id>` per ADR-0011.
    pub intent_key: String,
    /// 64-char lowercase-hex SHA-256 of the canonical rkyv-archived
    /// `Job` bytes per ADR-0002.
    pub spec_digest: String,
    /// Idempotency outcome echoed by the control plane.
    pub outcome: IdempotencyOutcome,
    /// Endpoint the POST was issued to.
    pub endpoint: Url,
    /// Next-command hint — `overdrive alloc status --job <workload_id>`.
    pub next_command: String,
    /// CLI exit code per ADR-0032 §9: 0 for `ConvergedRunning`, 1 for
    /// `ConvergedFailed`. Mapping of pre-Accepted errors → 2 lives in
    /// [`crate::render::cli_error_to_exit_code`].
    pub exit_code: i32,
    /// Operator-facing rendered text written to stdout — the success
    /// summary for `Running`, or the structured `Error:` block for
    /// `Failed`.
    pub summary: String,
    /// Terminal-reason payload from `ConvergedFailed`. `None` on the
    /// happy path (`ConvergedRunning`).
    pub terminal_reason: Option<TerminalReason>,
    /// Last cause-class `TransitionReason` observed on the broadcast
    /// bus before terminal — typically the most recent
    /// `LifecycleTransition.reason` carrying a failure variant. `None`
    /// when no failure transitions were observed.
    pub streaming_reason: Option<TransitionReason>,
    /// Verbatim driver error text from the `ConvergedFailed.error`
    /// field. `None` on the happy path.
    pub streaming_error: Option<String>,
}

/// Submit a job spec via the streaming NDJSON lane and consume to
/// terminal.
///
/// Per slice 02 step 02-04 acceptance criteria, this handler:
///
/// 1. Reads + validates the spec client-side via
///    [`Job::from_submit`] (ADR-0011) — fast-fail BEFORE any HTTP call.
/// 2. POSTs `application/x-ndjson` via
///    [`crate::http_client::ApiClient::submit_workload_streaming`].
/// 3. Consumes the response body line-by-line via
///    `reqwest::Response::bytes_stream()` + a `BytesMut`-backed line
///    splitter that tolerates partial chunks crossing recv boundaries.
/// 4. Deserialises each line into [`SubmitEvent`] and matches on the
///    event kind — `Accepted` populates the output prefix; lifecycle
///    transitions accumulate the latest cause-class reason; terminal
///    events compute the rendered summary + exit code and return.
///
/// # Errors
///
/// Same shapes as [`submit`] — pre-Accepted failures bubble up as
/// [`CliError`] variants. Once `Accepted` arrives this function does
/// not return `Err` for terminal failures: a `ConvergedFailed` event
/// is a successful termination of the stream that maps to exit code 1
/// via [`SubmitStreamingOutput::exit_code`].
///
/// # Panics
///
/// Does not panic on its own. The internal `expect("ApiClient::base_url")`
/// is unreachable — `from_config` returns `Err(CliError::ConfigLoad)`
/// on URL-parse failure, never returning a client whose base URL is
/// absent.
pub async fn submit_streaming(args: SubmitArgs) -> Result<SubmitStreamingOutput, CliError> {
    // 1. Read TOML from disk — same as the one-shot lane.
    let toml_str = std::fs::read_to_string(&args.spec).map_err(|e| CliError::InvalidSpec {
        field: "spec".to_string(),
        message: format!("failed to read `{}`: {e}", args.spec.display()),
    })?;

    // 2. Slice 02 of `workload-kind-discriminator`: parse via the
    //    kind-discriminating `WorkloadSpecInput::from_toml_str` driving
    //    port (ADR-0047 §2). Section presence in the TOML body
    //    (`[service]` / `[job]` / `[schedule]`) selects the variant.
    //    The legacy flat `JobSpecInput` parser is retained as the
    //    fallback for back-compat fixtures that don't yet carry a
    //    discriminator section — slice 02 wires the discriminator
    //    into production while preserving the legacy ingestion path
    //    for unmigrated tests until they are converted.
    let workload_input = WorkloadSpecInput::from_toml_str(&toml_str);

    if let Ok(WorkloadSpecInput::Job(job_spec)) = workload_input {
        // Job-kind dispatch (slice 02) — runs to completion; no
        // ConvergedRunning rendering reachable. Service / Schedule /
        // Err(_) cases all fall through to the legacy flat
        // `JobSpecInput` ingestion path: the control plane server-side
        // spec ingest accepts the same TOML shape; re-parse via
        // `JobSpecInput` to construct the legacy wire payload. This
        // bridge dies when slices 03+ migrate the wire format
        // end-to-end.
        return submit_streaming_job(args, job_spec).await;
    }

    // 3. Legacy path: flat `JobSpecInput` parser.
    let spec_input: JobSpecInput =
        toml::from_str(&toml_str).map_err(|e| CliError::InvalidSpec {
            field: "toml".to_string(),
            message: format!("failed to parse TOML: {e}"),
        })?;

    // 4. Client-side validation (ADR-0011 SSOT). Capture the validated
    //    `WorkloadId` so the streaming consumer can carry the canonical
    //    workload_id without re-parsing the server's `intent_key`.
    let validated: Job = Job::from_submit(spec_input.clone()).map_err(aggregate_to_cli_error)?;
    let validated_job_id = validated.id.to_string();

    // 5. Build the typed API client and POST with `Accept: application/x-ndjson`.
    let client = ApiClient::from_config(&args.config_path)?;
    let endpoint = client.base_url().clone();
    // Legacy lane — wrapped in `SubmitSpecInput::Job(_)` per ADR-0051.
    // The wire-side discriminator now lives inside `SubmitSpecInput`'s
    // `kind` tag; the outer `workload_kind` field has been deleted.
    let request = SubmitWorkloadRequest { spec: SubmitSpecInput::Job(spec_input) };
    let response = client.submit_workload_streaming(request).await?;

    // 6. Consume the stream line-by-line.
    consume_stream(response, endpoint, validated_job_id).await
}

/// Submit a Job-kind spec via the streaming NDJSON lane and consume to
/// terminal. Per ADR-0047 §3 [D2] / [D7]: Job kind has run-to-
/// completion semantics — `ConvergedRunning` is structurally
/// unreachable; the terminal verdict is `Succeeded` (exit 0) or
/// `Failed` (non-zero exit code or backoff exhausted). The CLI
/// process exit code equals the workload's kernel-observed exit code.
async fn submit_streaming_job(
    args: SubmitArgs,
    job_spec: JobSpec,
) -> Result<SubmitStreamingOutput, CliError> {
    // Translate the kind-discriminated `JobSpec` to the legacy
    // `JobSpecInput` wire shape the server's spec-ingest still
    // expects (server-side `WorkloadSpec` ingest is the next slice's
    // work). The translation is mechanical: the `JobSpec` already
    // carries the same fields — id, exec, resources.
    // The kind-discriminator parser produces `ExecInput`/`ResourcesInput`
    // types living in `aggregate::workload_spec`; the legacy `JobSpecInput`
    // wire shape uses the same-named types in `aggregate::mod`. The
    // shapes are field-identical; project field-by-field.
    let spec_input = JobSpecInput {
        id: job_spec.id,
        replicas: 1,
        driver: DriverInput::Exec(LegacyExecInput {
            command: job_spec.exec.command,
            args: job_spec.exec.args,
        }),
        resources: LegacyResourcesInput {
            cpu_milli: job_spec.resources.cpu_milli,
            memory_bytes: job_spec.resources.memory_bytes,
        },
    };

    // Client-side validation via the shared ADR-0011 constructor.
    let validated: Job = Job::from_submit(spec_input.clone()).map_err(aggregate_to_cli_error)?;
    let validated_job_id = validated.id.to_string();

    // Submit echo (per S-02-06) — printed via stdout BEFORE any
    // streaming events so the operator sees the kind upfront. The
    // legacy code path renders this as part of the terminal summary
    // at present; the post-Accepted prefix is accumulated into the
    // final summary string returned by the handler so the CLI
    // wrapper prints it verbatim.
    let submit_echo = crate::render::format_job_submit_echo(&validated_job_id);

    let client = ApiClient::from_config(&args.config_path)?;
    let endpoint = client.base_url().clone();
    // Per ADR-0051 the wire-side workload kind is the `kind` tag
    // inside `SubmitSpecInput::Job(_)`. The outer
    // `workload_kind: Option<String>` field has been deleted; the
    // server dispatches to `build_workload_stream` (typed
    // `JobSubmitEvent` lane) based on the discriminator persisted at
    // `IntentKey::for_workload_kind` after admission.
    let request = SubmitWorkloadRequest { spec: SubmitSpecInput::Job(spec_input) };
    let response = client.submit_workload_streaming(request).await?;

    consume_stream_job(response, endpoint, validated_job_id, submit_echo).await
}

/// Drive the NDJSON stream from `response` to a terminal event and
/// produce the typed [`SubmitStreamingOutput`].
///
/// The line splitter accumulates bytes into a `BytesMut` and yields
/// each newline-terminated line into `serde_json::from_slice` —
/// tolerating partial chunks that cross `recv` boundaries.
// The streaming consumer's body is naturally long — one event matcher
// per `SubmitEvent` variant plus the terminal-event projection logic.
// Splitting helpers per-arm would obscure the linear "for each line,
// match the variant" shape that makes the loop comprehensible. The
// 108-line function compares to the 100-line clippy default.
#[allow(clippy::too_many_lines)]
async fn consume_stream(
    response: reqwest::Response,
    endpoint: Url,
    validated_job_id: String,
) -> Result<SubmitStreamingOutput, CliError> {
    let mut stream = response.bytes_stream();
    let mut buf = BytesMut::new();

    // Stream-start wall-clock — used to compute the converged-running
    // summary duration in place of the historical `"live"` literal
    // (US-06 of `workload-kind-discriminator`). The CLI is a `binary`
    // crate so `Instant::now()` is allowed by dst-lint; the value is
    // used only for operator-facing display.
    let stream_started = std::time::Instant::now();

    // State accumulated as the stream proceeds. Populated by the
    // `Accepted` event (first) and by intermediate `LifecycleTransition`
    // events; consulted at terminal time to build the typed output.
    let mut accepted: Option<AcceptedFields> = None;
    let mut latest_cause_class_reason: Option<TransitionReason> = None;

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| CliError::Transport {
            endpoint: endpoint.to_string(),
            cause: format!("stream chunk error: {e}"),
        })?;
        buf.extend_from_slice(&chunk);

        // Drain every complete line currently in the buffer.
        while let Some(newline_pos) = buf.iter().position(|&b| b == b'\n') {
            let line = buf.split_to(newline_pos + 1);
            // Drop the trailing newline before deserialisation.
            let line_bytes = &line[..line.len() - 1];
            // Skip blank keep-alive lines if any.
            if line_bytes.is_empty() {
                continue;
            }
            let event: SubmitEvent =
                serde_json::from_slice(line_bytes).map_err(|e| CliError::BodyDecode {
                    cause: format!(
                        "failed to deserialise NDJSON line as SubmitEvent: {e}; \
                         line bytes: {}",
                        String::from_utf8_lossy(line_bytes)
                    ),
                })?;

            match event {
                SubmitEvent::Accepted { spec_digest, intent_key, outcome } => {
                    // The server-derived `intent_key` carries the
                    // canonical `IntentKey` shape; the CLI uses the
                    // already-validated client-side `WorkloadId` (captured
                    // before the POST) as the operator-facing `workload_id`
                    // so the SSOT for the `jobs/` prefix literal stays
                    // in `overdrive_core::aggregate::IntentKey::for_job`
                    // (the `intent_key_canonical` gate enforces this).
                    accepted = Some(AcceptedFields {
                        workload_id: validated_job_id.clone(),
                        intent_key,
                        spec_digest,
                        outcome,
                    });
                }
                SubmitEvent::LifecycleTransition { reason, .. } => {
                    // Accumulate the latest cause-class reason so we have
                    // it on terminal time for byte-equality assertions
                    // (S-WS-02 KPI-02).
                    if reason.is_failure() {
                        latest_cause_class_reason = Some(reason);
                    }
                }
                SubmitEvent::ConvergedRunning { alloc_id: _, started_at: _ } => {
                    let acc = accepted.ok_or_else(|| CliError::BodyDecode {
                        cause: "ConvergedRunning before Accepted on the streaming bus".to_string(),
                    })?;
                    let took_human = crate::render::format_human_duration(stream_started.elapsed());
                    let summary = crate::render::format_running_summary(
                        &acc.workload_id,
                        // Phase-1 single-replica streaming witness — the
                        // first `Running` row terminates the stream per
                        // architecture.md §10. Replica counts are an
                        // observation-side concern (alloc status); the
                        // streaming surface signals "the job has reached
                        // running" via the terminal event.
                        1,
                        1,
                        &took_human,
                    );
                    let next_command = format!("overdrive alloc status --job {}", acc.workload_id);
                    return Ok(SubmitStreamingOutput {
                        workload_id: acc.workload_id,
                        intent_key: acc.intent_key,
                        spec_digest: acc.spec_digest,
                        outcome: acc.outcome,
                        endpoint,
                        next_command,
                        exit_code: 0,
                        summary,
                        terminal_reason: None,
                        streaming_reason: None,
                        streaming_error: None,
                    });
                }
                SubmitEvent::ConvergedFailed { alloc_id: _, terminal_reason, reason, error } => {
                    let acc = accepted.ok_or_else(|| CliError::BodyDecode {
                        cause: "ConvergedFailed before Accepted on the streaming bus".to_string(),
                    })?;
                    // Prefer standalone reason; fall back to the latest
                    // cause-class transition seen on the bus so the
                    // KPI-02 byte-equality assertion has a stable
                    // source. `or_else` defers the fallback `Option`
                    // construction so the `latest_cause_class_reason`
                    // clone fires only when `reason` is `None`.
                    let stream_reason = reason.or_else(|| latest_cause_class_reason.clone());
                    let summary = crate::render::format_failed_block(
                        &acc.workload_id,
                        stream_reason.as_ref(),
                        error.as_deref(),
                        &terminal_reason,
                    );
                    let next_command = format!("overdrive alloc status --job {}", acc.workload_id);
                    return Ok(SubmitStreamingOutput {
                        workload_id: acc.workload_id,
                        intent_key: acc.intent_key,
                        spec_digest: acc.spec_digest,
                        outcome: acc.outcome,
                        endpoint,
                        next_command,
                        exit_code: 1,
                        summary,
                        terminal_reason: Some(terminal_reason),
                        streaming_reason: stream_reason,
                        streaming_error: error,
                    });
                }
                // Terminal — workload reached a clean stop (operator
                // intent, reconciler convergence, or natural process
                // exit). Maps to exit code 0 across all `StoppedBy`
                // variants per RCA + ADR-0032 §9: clean stop is a
                // successful terminal outcome from the operator's
                // perspective. RCA:
                // `docs/feature/fix-converged-stopped-cli-arm/deliver/rca.md`.
                SubmitEvent::ConvergedStopped { alloc_id: _, by } => {
                    // mutants::skip — arm exercised by streaming_submit_converged_stopped integration test; HTTP-stream consumer not unit-testable
                    let acc = accepted.ok_or_else(|| CliError::BodyDecode {
                        cause: "ConvergedStopped before Accepted on the streaming bus".to_string(),
                    })?;
                    // Slice 04 (`workload-kind-discriminator`) — the legacy
                    // long-running streaming submit path is semantically a
                    // Service workload. Slice 02 will wire the WorkloadSpec
                    // discriminator into submit_streaming and pass the
                    // parsed kind here verbatim; until then the legacy
                    // flat-shape parser produces what is conceptually a
                    // Service, so we hard-code Service vocabulary.
                    let summary = crate::render::format_stopped_summary(
                        &acc.workload_id,
                        overdrive_core::aggregate::WorkloadKind::Service,
                        by,
                    );
                    let next_command = format!("overdrive alloc status --job {}", acc.workload_id);
                    return Ok(SubmitStreamingOutput {
                        workload_id: acc.workload_id,
                        intent_key: acc.intent_key,
                        spec_digest: acc.spec_digest,
                        outcome: acc.outcome,
                        endpoint,
                        next_command,
                        exit_code: 0,
                        summary,
                        terminal_reason: None,
                        streaming_reason: None,
                        streaming_error: None,
                    });
                }
                // `SubmitEvent` is `#[non_exhaustive]` — future variants
                // are observed and ignored until the consumer grows
                // explicit handling. Logged via tracing so an operator
                // running with `RUST_LOG=info` sees the unfamiliar
                // event without the stream stalling.
                _ => {
                    tracing::debug!("ignoring unrecognised SubmitEvent variant on stream");
                }
            }
        }
    }

    // Stream closed without a terminal event — protocol violation.
    Err(CliError::BodyDecode {
        cause: "streaming submit response closed without a terminal event \
                (ConvergedRunning, ConvergedFailed, or ConvergedStopped)"
            .to_string(),
    })
}

/// Drive a Job-kind streaming submit to terminal — slice 02 of
/// `workload-kind-discriminator`.
/// Per ADR-0047 §3 [D2]: Job-kind has run-to-completion semantics.
/// Unlike Service-kind, `ConvergedRunning` is NOT a terminal event;
/// the terminal verdict is `Succeeded` (workload exit 0) or `Failed`
/// (non-zero exit code observed). The CLI process exit code equals
/// the workload's kernel-observed exit code per KPI K1.
///
/// Per slice 02-06 the wire format is the typed sibling-event enum
/// [`JobSubmitEvent`] (ADR-0047 §3 [D7]); `ConvergedRunning` is
/// structurally absent on this code path because the type carries
/// no equivalent variant. This consumer projects
/// `JobSubmitEvent::Succeeded` → `format_job_succeeded_summary`,
/// `JobSubmitEvent::Failed` → `format_job_failed_summary`,
/// `JobSubmitEvent::Stopped` → `format_job_stopped_summary`, and
/// `JobSubmitEvent::AttemptFailed` → intermediate per-attempt line
/// (stream stays open).
//
// The Job-kind streaming consumer is naturally long — one event
// matcher per `JobSubmitEvent` variant plus the per-arm projection
// logic. Mirrors the existing `consume_stream` consumer's line-budget
// exemption for the same reason.
#[allow(clippy::too_many_lines)]
async fn consume_stream_job(
    response: reqwest::Response,
    endpoint: Url,
    validated_job_id: String,
    submit_echo: String,
) -> Result<SubmitStreamingOutput, CliError> {
    let mut stream = response.bytes_stream();
    let mut buf = BytesMut::new();
    let stream_started = std::time::Instant::now();

    let mut accepted: Option<AcceptedFields> = None;
    // Per slice 02-06 / S-02-03: the Job-kind streaming surface emits
    // intermediate `AttemptFailed` events between attempts. We
    // accumulate the operator-facing intermediate lines into the
    // running summary so the operator sees the per-attempt narrative
    // and the terminal verdict in one buffer; the stream stays open
    // across `AttemptFailed` and closes only on `Succeeded` / `Failed` / `Stopped`.
    let mut summary = submit_echo;

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| CliError::Transport {
            endpoint: endpoint.to_string(),
            cause: format!("stream chunk error: {e}"),
        })?;
        buf.extend_from_slice(&chunk);

        while let Some(newline_pos) = buf.iter().position(|&b| b == b'\n') {
            let line = buf.split_to(newline_pos + 1);
            // mutants::skip — `-` → `/` is `len/1 == len`; serde_json tolerates trailing whitespace so mutation is a behavioral no-op
            let line_bytes = &line[..line.len() - 1];
            if line_bytes.is_empty() {
                continue;
            }
            let event: JobSubmitEvent =
                serde_json::from_slice(line_bytes).map_err(|e| CliError::BodyDecode {
                    cause: format!(
                        "failed to deserialise NDJSON line as JobSubmitEvent: {e}; line bytes: {}",
                        String::from_utf8_lossy(line_bytes)
                    ),
                })?;

            match event {
                JobSubmitEvent::Accepted { spec_digest, intent_key, outcome } => {
                    accepted = Some(AcceptedFields {
                        workload_id: validated_job_id.clone(),
                        intent_key,
                        spec_digest,
                        outcome,
                    });
                }
                // Pending / Running are informational — Per ADR-0047 §3
                // [D2] Job-kind workloads are run-to-completion; the
                // stream waits for the terminal verdict and these
                // events do not produce per-variant render lines.
                JobSubmitEvent::Pending | JobSubmitEvent::Running { .. } => {
                    // mutants::skip — informational arm; deletion falls to wildcard producing identical trace-only behavior
                    tracing::debug!("Job stream informational event; awaiting terminal");
                }
                // S-02-03: AttemptFailed is intermediate — the stream
                // stays open. Render the operator-facing line and
                // continue to the next event.
                // mutants::skip — exercised by job_kind_streaming integration test; HTTP-stream consumer not unit-testable
                JobSubmitEvent::AttemptFailed {
                    attempt_index,
                    exit_code,
                    next_attempt_delay,
                    ..
                } => {
                    let acc_ref = accepted.as_ref().ok_or_else(|| CliError::BodyDecode {
                        cause: "AttemptFailed before Accepted on the streaming bus".to_string(),
                    })?;
                    let delay = next_attempt_delay.as_deref().unwrap_or("0ms");
                    summary.push_str(&crate::render::format_job_attempt_failed(
                        &acc_ref.workload_id,
                        attempt_index,
                        exit_code,
                        delay,
                    ));
                }
                JobSubmitEvent::Succeeded { exit_code, attempts, .. } => {
                    let acc = accepted.ok_or_else(|| CliError::BodyDecode {
                        cause: "Succeeded before Accepted on the streaming bus".to_string(),
                    })?;
                    let took_human = crate::render::format_human_duration(stream_started.elapsed());
                    summary.push_str(&crate::render::format_job_succeeded_summary(
                        &acc.workload_id,
                        exit_code,
                        &took_human,
                        attempts,
                    ));
                    let next_command = format!("overdrive alloc status --job {}", acc.workload_id);
                    return Ok(SubmitStreamingOutput {
                        workload_id: acc.workload_id,
                        intent_key: acc.intent_key,
                        spec_digest: acc.spec_digest,
                        outcome: acc.outcome,
                        endpoint,
                        next_command,
                        // Per KPI K1 honesty: CLI process exit code =
                        // workload kernel exit code. For Succeeded,
                        // the wire-side `exit_code` carries the
                        // observed kernel exit code (canonically 0
                        // but the variant carries the value verbatim
                        // for forward-compat with non-zero "successes"
                        // a future reconciler may stamp).
                        exit_code,
                        summary,
                        terminal_reason: None,
                        streaming_reason: None,
                        streaming_error: None,
                    });
                }
                JobSubmitEvent::Stopped { stopped_by, attempts, .. } => {
                    let acc = accepted.ok_or_else(|| CliError::BodyDecode {
                        cause: "Stopped before Accepted on the streaming bus".to_string(),
                    })?;
                    let took_human = crate::render::format_human_duration(stream_started.elapsed());
                    let initiator = match stopped_by {
                        overdrive_core::transition_reason::StoppedBy::Operator => "operator",
                        overdrive_core::transition_reason::StoppedBy::Reconciler => "reconciler",
                        _ => "system",
                    };
                    summary.push_str(&crate::render::format_job_stopped_summary(
                        &acc.workload_id,
                        initiator,
                        &took_human,
                        attempts,
                    ));
                    let next_command = format!("overdrive alloc status --job {}", acc.workload_id);
                    return Ok(SubmitStreamingOutput {
                        workload_id: acc.workload_id,
                        intent_key: acc.intent_key,
                        spec_digest: acc.spec_digest,
                        outcome: acc.outcome,
                        endpoint,
                        next_command,
                        exit_code: 130,
                        summary,
                        terminal_reason: None,
                        streaming_reason: None,
                        streaming_error: None,
                    });
                }
                JobSubmitEvent::Failed {
                    exit_code, attempts, max_attempts, stderr_tail, ..
                } => {
                    let acc = accepted.ok_or_else(|| CliError::BodyDecode {
                        cause: "Failed before Accepted on the streaming bus".to_string(),
                    })?;
                    let took_human = crate::render::format_human_duration(stream_started.elapsed());
                    let backoff_exhausted =
                        crate::render::is_backoff_exhausted(attempts, max_attempts);
                    let stderr_str = stderr_tail.clone().unwrap_or_default();
                    summary.push_str(&crate::render::format_job_failed_summary(
                        &acc.workload_id,
                        exit_code,
                        &took_human,
                        attempts,
                        max_attempts,
                        backoff_exhausted,
                        &stderr_str,
                    ));
                    let next_command = format!("overdrive alloc status --job {}", acc.workload_id);
                    return Ok(SubmitStreamingOutput {
                        workload_id: acc.workload_id,
                        intent_key: acc.intent_key,
                        spec_digest: acc.spec_digest,
                        outcome: acc.outcome,
                        endpoint,
                        next_command,
                        exit_code,
                        summary,
                        terminal_reason: None,
                        streaming_reason: None,
                        streaming_error: stderr_tail,
                    });
                }
                // `JobSubmitEvent` is `#[non_exhaustive]` — preserve
                // forward-compat. Unknown variants are debug-logged
                // and ignored; the stream waits for a known terminal.
                _ => {
                    tracing::debug!("ignoring unrecognised JobSubmitEvent variant on Job stream");
                }
            }
        }
    }

    Err(CliError::BodyDecode {
        cause: "Job streaming submit response closed without a terminal event".to_string(),
    })
}

/// Internal accumulator for the `SubmitEvent::Accepted` fields, used to
/// build [`SubmitStreamingOutput`] when the terminal event arrives.
struct AcceptedFields {
    workload_id: String,
    intent_key: String,
    spec_digest: String,
    outcome: IdempotencyOutcome,
}

/// Map [`AggregateError`] (from `overdrive_core`) into a
/// [`CliError::InvalidSpec`] with the offending field name and a
/// human-readable reason. Separate from the HTTP-lane 400 mapping —
/// this is strictly client-side, pre-HTTP.
fn aggregate_to_cli_error(err: AggregateError) -> CliError {
    match err {
        AggregateError::Validation { field, message } => {
            CliError::InvalidSpec { field: field.to_string(), message }
        }
        AggregateError::Id(id_err) => {
            CliError::InvalidSpec { field: "id".to_string(), message: id_err.to_string() }
        }
        AggregateError::Resources(msg) => {
            CliError::InvalidSpec { field: "resources".to_string(), message: msg }
        }
    }
}
