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
use overdrive_control_plane::api::{IdempotencyOutcome, StopOutcome, SubmitWorkloadRequest};
use overdrive_control_plane::streaming::JobSubmitEvent;
use overdrive_core::TransitionReason;
use overdrive_core::aggregate::{
    AggregateError, DriverInput, ExecInput as LegacyExecInput, IntentKey, Job, JobSpec,
    JobSpecInput, ParseError, ResourcesInput as LegacyResourcesInput, ServiceSpec, ServiceV1,
    WorkloadSpecInput,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
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
    //    translate to the wire shape via `JobSpecInput`. A `[service]`
    //    body (with `[[listener]]` blocks) routes to the Service-kind
    //    deploy lane below — this is the non-streaming (JSON-ack)
    //    companion to `submit_streaming_service`, used by the detached /
    //    non-TTY `overdrive deploy` path (`main.rs` `Command::Deploy`).
    //    Flat TOMLs fall through to the legacy `JobSpecInput` parser.
    //    Per ADR-0051 the wire-side discriminator lives inside
    //    `SubmitSpecInput`'s `kind` tag.
    let spec_input: JobSpecInput = match WorkloadSpecInput::from_toml_str(&toml_str) {
        Ok(WorkloadSpecInput::Job(job_spec)) => JobSpecInput {
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
        },
        // Service-kind deploy in the JSON-ack lane. Routes to the
        // Service submit path so the listener protocol (e.g. udp)
        // threads through to the persisted `WorkloadIntent::Service`
        // intent. Without this arm a `[service]` spec falls through to
        // the legacy `JobSpecInput` parser and is rejected as
        // "missing field `id`" — the gap udp-service-support step 01-05
        // closes (the deploy half of S-04-A).
        Ok(WorkloadSpecInput::Service(service_spec)) => {
            return submit_service(args, service_spec).await;
        }
        // Slice 07 / US-07 — surface the kind-rejection verbatim with
        // its per-kind guidance. Without this explicit arm the error
        // would be swallowed by the legacy `toml::from_str` fall-through
        // below, hiding the teaching message from the operator.
        Err(parse_err @ ParseError::ProbesNotAllowedOnKind { .. }) => {
            return Err(CliError::ParseError(parse_err));
        }
        // Schedule kind and other parse failures fall through to the
        // legacy flat `JobSpecInput` parser — unchanged behaviour.
        Ok(_) | Err(_) => toml::from_str(&toml_str).map_err(|e| CliError::InvalidSpec {
            field: "toml".to_string(),
            message: format!("failed to parse TOML: {e}"),
        })?,
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

/// Service-kind deploy in the JSON-ack (non-streaming) lane — the
/// companion to [`submit_streaming_service`] for the detached / non-TTY
/// `overdrive deploy` path.
///
/// Projects the parser-side [`ServiceSpec`] (carries `Listener` with a
/// `NonZeroU16` port and the `Proto` newtype) to the wire-side
/// [`ServiceSpecInput`] (`u16` port + `String` protocol for JSON
/// tolerance) and POSTs as `SubmitSpecInput::Service(_)`. The listener
/// protocol threads through unchanged: the server's
/// `ServiceV1::from_submit` re-parses the `String` token back into
/// `Proto`, so the persisted `WorkloadIntent::Service(ServiceV1)`
/// carries the operator's declared protocol verbatim.
///
/// Returns the same [`SubmitOutput`] shape as the Job lane so the
/// caller renders `workload_submit_accepted` identically across kinds.
async fn submit_service(
    args: SubmitArgs,
    service_spec: ServiceSpec,
) -> Result<SubmitOutput, CliError> {
    // Project parser-side `ServiceSpec` → wire-side `ServiceSpecInput`,
    // mirroring `submit_streaming_service`. The listener `(NonZeroU16,
    // Proto)` pair projects to `(u16, String)`; the protocol's canonical
    // lowercase render (`Proto::as_str`) is what the server re-parses.
    let listeners: Vec<ListenerInput> = service_spec
        .listeners
        .iter()
        .map(|l| ListenerInput { port: l.port.get(), protocol: l.protocol.as_str().to_owned() })
        .collect();
    let spec_input = ServiceSpecInput {
        id: service_spec.id,
        replicas: service_spec.replicas,
        resources: LegacyResourcesInput {
            cpu_milli: service_spec.resources.cpu_milli,
            memory_bytes: service_spec.resources.memory_bytes,
        },
        driver: DriverInput::Exec(LegacyExecInput {
            command: service_spec.exec.command,
            args: service_spec.exec.args,
        }),
        listeners,
        startup_probes: service_spec.startup_probes,
        readiness_probes: service_spec.readiness_probes,
        liveness_probes: service_spec.liveness_probes,
    };

    // Client-side validation via the shared ADR-0011 constructor —
    // same fast-fail discipline as the Job lane's `Job::from_submit`.
    let _validated: ServiceV1 =
        ServiceV1::from_submit(spec_input.clone()).map_err(aggregate_to_cli_error)?;

    let client = ApiClient::from_config(&args.config_path)?;
    let endpoint = client.base_url().clone();
    let resp = client
        .submit_workload(SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec_input) })
        .await?;

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
/// `Succeeded` or `Failed` event arrives. The handler
/// returns this typed shape carrying:
///
///  * `Accepted`-event-derived fields (`workload_id`, `intent_key`,
///    `spec_digest`, `outcome`) — same shape as the one-shot ack lane
///    so existing renderer/tests keep their assertion shapes.
///  * `exit_code` — 0 on `Succeeded`, non-zero on `Failed`. The
///    binary wrapper at `main.rs` surfaces this as the process exit
///    code, satisfying ADR-0032 §9.
///  * `summary` — operator-facing rendered text written to stdout (the
///    success summary line for `Succeeded`, the structured `Error:` block
///    for `Failed`).
///  * `streaming_reason` / `streaming_error` —
///    typed projections of the terminal event payloads, used
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
    /// CLI exit code per ADR-0032 §9: 0 for `Succeeded`, the workload's
    /// kernel-observed exit code for `Failed`. Mapping of pre-Accepted
    /// errors → 2 lives in [`crate::render::cli_error_to_exit_code`].
    pub exit_code: i32,
    /// Operator-facing rendered text written to stdout — the success
    /// summary for `Succeeded`, or the structured `Error:` block for
    /// `Failed`.
    pub summary: String,
    /// Last cause-class `TransitionReason` observed on the broadcast
    /// bus before terminal — typically the most recent failure-carrying
    /// lifecycle transition reason. `None`
    /// when no failure transitions were observed.
    pub streaming_reason: Option<TransitionReason>,
    /// Verbatim driver error text from the terminal `Failed` event.
    /// `None` on the happy path.
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
/// 4. Deserialises each line into the per-kind streaming event enum
///    (`JobSubmitEvent` / `ServiceSubmitEvent`) and matches on the
///    event kind — `Accepted` populates the output prefix; lifecycle
///    transitions accumulate the latest cause-class reason; terminal
///    events compute the rendered summary + exit code and return.
///
/// # Errors
///
/// Same shapes as [`submit`] — pre-Accepted failures bubble up as
/// [`CliError`] variants. Once `Accepted` arrives this function does
/// not return `Err` for terminal failures: a `Failed` event
/// is a successful termination of the stream that maps to a non-zero
/// exit code via [`SubmitStreamingOutput::exit_code`].
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

    // 2. Parse via the kind-discriminating `WorkloadSpecInput::from_toml_str`
    //    driving port (ADR-0047 §2). Section presence in the TOML body
    //    (`[service]` / `[job]` / `[schedule]`) selects the variant. Per
    //    step 01-03e3-fix the dispatch is per-kind: each arm wraps its
    //    own typed payload into the matching `SubmitSpecInput::*` variant
    //    and routes to a per-kind streaming consumer (Job →
    //    `submit_streaming_job` → `JobSubmitEvent` arms; Service →
    //    `submit_streaming_service` → `ServiceSubmitEvent` arms). The
    //    legacy "fall through to flat `JobSpecInput` + wrap in
    //    `SubmitSpecInput::Job`" path was the gap that produced the
    //    cross-routing bug 01-03e3-fix closes: Service-kind TOML must
    //    NEVER be wrapped in `SubmitSpecInput::Job(_)`.
    match WorkloadSpecInput::from_toml_str(&toml_str) {
        Ok(WorkloadSpecInput::Job(job_spec)) => submit_streaming_job(args, job_spec).await,
        Ok(WorkloadSpecInput::Service(service_spec)) => {
            submit_streaming_service(args, service_spec).await
        }
        Ok(WorkloadSpecInput::Schedule(_)) => Err(CliError::InvalidSpec {
            field: "spec.kind".to_string(),
            message: "schedule submission is not yet implemented (ADR-0051 OQ-5)".to_string(),
        }),
        Err(parse_err) => Err(CliError::InvalidSpec {
            field: "toml".to_string(),
            message: format!("failed to parse workload spec: {parse_err}"),
        }),
    }
}

/// Submit a Job-kind spec via the streaming NDJSON lane and consume to
/// terminal. Per ADR-0047 §3 [D2] / [D7]: Job kind has run-to-
/// completion semantics — `Running` is informational and never
/// terminal; the terminal verdict is `Succeeded` (exit 0) or
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

/// Submit a Service-kind spec via the streaming NDJSON lane and consume
/// to terminal. Mirrors [`submit_streaming_job`] for the Service kind
/// per ADR-0047 §3 [D2] / [D7] and step 01-03e3-fix: routes to the
/// `ServiceSubmitEvent` consumer surface (`Accepted / Stable / Failed /
/// Stopped`), NOT the `JobSubmitEvent` surface.
///
/// Projects the parser-side [`ServiceSpec`] (carries `Listener` with
/// `NonZeroU16` port and `Proto` enum) to the wire-side
/// [`ServiceSpecInput`] (carries `ListenerInput` with `u16` port and
/// `String` protocol) and POSTs as `SubmitSpecInput::Service(_)`.
async fn submit_streaming_service(
    args: SubmitArgs,
    service_spec: ServiceSpec,
) -> Result<SubmitStreamingOutput, CliError> {
    // Project parser-side `ServiceSpec` → wire-side `ServiceSpecInput`.
    // The parser-side `Listener` carries `(NonZeroU16, Proto)`; the
    // wire-side `ListenerInput` carries `(u16, String)` for JSON
    // tolerance. Both sides go through `ServiceV1::from_submit` server-
    // side; the client-side fast-fail validation below also exercises
    // the same constructor for symmetry with the Job-kind lane.
    let listeners: Vec<ListenerInput> = service_spec
        .listeners
        .iter()
        .map(|l| ListenerInput { port: l.port.get(), protocol: l.protocol.as_str().to_owned() })
        .collect();
    // Probe descriptors project through unchanged — the parser
    // populates `service_spec.startup_probes` from the TOML
    // `[[health_check.startup]]` blocks (plus default-TCP inference
    // per ADR-0058); the wire envelope carries them through to
    // `ServiceV1::from_submit` server-side. Readiness / liveness
    // probe vecs are reserved for future slices (02-01 / 02-02)
    // and pass through as the empty vecs the parser populates.
    let spec_input = ServiceSpecInput {
        id: service_spec.id,
        replicas: service_spec.replicas,
        resources: LegacyResourcesInput {
            cpu_milli: service_spec.resources.cpu_milli,
            memory_bytes: service_spec.resources.memory_bytes,
        },
        driver: DriverInput::Exec(LegacyExecInput {
            command: service_spec.exec.command,
            args: service_spec.exec.args,
        }),
        listeners,
        startup_probes: service_spec.startup_probes,
        readiness_probes: service_spec.readiness_probes,
        liveness_probes: service_spec.liveness_probes,
    };

    // Client-side validation via the shared ADR-0011 constructor — same
    // discipline as the Job-kind lane's `Job::from_submit` fast-fail.
    let validated: ServiceV1 =
        ServiceV1::from_submit(spec_input.clone()).map_err(aggregate_to_cli_error)?;
    let validated_workload_id = validated.id.to_string();

    let client = ApiClient::from_config(&args.config_path)?;
    let endpoint = client.base_url().clone();
    // Per ADR-0047 §3 / ADR-0051 the Service-kind wire-side payload is
    // `SubmitSpecInput::Service(_)`. The handler dispatches to
    // `streaming::build_service_stream` (typed `ServiceSubmitEvent`
    // lane per step 01-03e3) based on the persisted discriminator.
    let request = SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec_input) };
    let response = client.submit_workload_streaming(request).await?;

    consume_stream(response, endpoint, validated_workload_id).await
}

/// Drive a Service-kind streaming submit to terminal.
///
/// Per ADR-0056 / ADR-0059 / step 01-03e3: the Service-kind wire
/// surface emits the typed [`ServiceSubmitEvent`] enum:
///   * `Accepted` — synchronous first line.
///   * `Stable` — terminal; CLI exit 0.
///   * `Failed` — terminal; CLI exit 1.
///   * `Stopped` — terminal; CLI exit 0.
#[allow(clippy::too_many_lines)]
async fn consume_stream(
    response: reqwest::Response,
    endpoint: Url,
    validated_job_id: String,
) -> Result<SubmitStreamingOutput, CliError> {
    use overdrive_control_plane::streaming::ServiceSubmitEvent;

    let mut stream = response.bytes_stream();
    let mut buf = BytesMut::new();
    let mut accepted: Option<AcceptedFields> = None;
    // Stream-side elapsed measurement for the Slice 08 EarlyExit
    // `elapsed:` render line (S-SHCP-CLI-07). Best-effort from the CLI's
    // own clock — the EarlyExit wire variant carries only `exit_code`,
    // so the elapsed/deadline are recomputed render-side per the
    // persist-inputs rule (never persisted on the variant).
    let stream_started = std::time::Instant::now();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| CliError::Transport {
            endpoint: endpoint.to_string(),
            cause: format!("stream chunk error: {e}"),
        })?;
        buf.extend_from_slice(&chunk);

        while let Some(newline_pos) = buf.iter().position(|&b| b == b'\n') {
            let line = buf.split_to(newline_pos + 1);
            let line_bytes = &line[..line.len() - 1];
            if line_bytes.is_empty() {
                continue;
            }
            let event: ServiceSubmitEvent =
                serde_json::from_slice(line_bytes).map_err(|e| CliError::BodyDecode {
                    cause: format!(
                        "failed to deserialise NDJSON line as ServiceSubmitEvent: {e}; \
                         line bytes: {}",
                        String::from_utf8_lossy(line_bytes)
                    ),
                })?;

            match event {
                ServiceSubmitEvent::Accepted { spec_digest, intent_key, outcome } => {
                    accepted = Some(AcceptedFields {
                        workload_id: validated_job_id.clone(),
                        intent_key,
                        spec_digest,
                        outcome,
                    });
                }
                ServiceSubmitEvent::Stable { alloc_id: _, settled_in_ms, witness } => {
                    let acc = accepted.ok_or_else(|| CliError::BodyDecode {
                        cause: "Stable before Accepted on the streaming bus".to_string(),
                    })?;
                    let summary = crate::render::format_service_stable_summary(
                        &acc.workload_id,
                        settled_in_ms,
                        &witness,
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
                        streaming_reason: None,
                        streaming_error: None,
                    });
                }
                ServiceSubmitEvent::Failed { alloc_id: _, reason, stderr_tail } => {
                    let acc = accepted.ok_or_else(|| CliError::BodyDecode {
                        cause: "Failed before Accepted on the streaming bus".to_string(),
                    })?;
                    // EarlyExit renders the `elapsed:` line from the
                    // stream-side measurement + the default startup
                    // deadline (the variant carries only `exit_code`).
                    let early_exit_timing = matches!(
                        reason,
                        overdrive_core::transition_reason::ServiceFailureReason::EarlyExit { .. }
                    )
                    .then(|| {
                        let deadline_secs =
                            overdrive_core::service_lifecycle::DEFAULT_STARTUP_DEADLINE.as_secs();
                        (stream_started.elapsed().as_secs(), deadline_secs)
                    });
                    let summary = crate::render::format_service_failed_block(
                        &acc.workload_id,
                        &reason,
                        stderr_tail.as_deref(),
                        early_exit_timing,
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
                        streaming_reason: None,
                        streaming_error: stderr_tail,
                    });
                }
                ServiceSubmitEvent::Stopped { alloc_id: _, by } => {
                    let acc = accepted.ok_or_else(|| CliError::BodyDecode {
                        cause: "Stopped before Accepted on the streaming bus".to_string(),
                    })?;
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
                        streaming_reason: None,
                        streaming_error: None,
                    });
                }
                _ => {
                    tracing::debug!(
                        "ignoring unrecognised ServiceSubmitEvent variant on Service stream"
                    );
                }
            }
        }
    }

    Err(CliError::BodyDecode {
        cause: "Service streaming submit response closed without a terminal event \
                (Stable, Failed, or Stopped)"
            .to_string(),
    })
}

/// Drive a Job-kind streaming submit to terminal — slice 02 of
/// `workload-kind-discriminator`.
/// Per ADR-0047 §3 [D2]: Job-kind has run-to-completion semantics.
/// Unlike Service-kind, `Running` is NOT a terminal event;
/// the terminal verdict is `Succeeded` (workload exit 0) or `Failed`
/// (non-zero exit code observed). The CLI process exit code equals
/// the workload's kernel-observed exit code per KPI K1.
///
/// Per slice 02-06 the wire format is the typed sibling-event enum
/// [`JobSubmitEvent`] (ADR-0047 §3 [D7]); a converged-running terminal
/// is structurally absent on this code path because the type carries
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
                        exit_code: exit_code.unwrap_or(1),
                        summary,
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

/// Internal accumulator for the streaming `Accepted` event fields, used to
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
