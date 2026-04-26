# Slice 5 — CLI handlers for job / alloc / cluster / node

**Story**: US-05
**Walking skeleton row**: 5 (Drive from the CLI)
**Effort**: ~1 day
**Depends on**: Slice 2 (REST service surface + HTTP client), Slice 3 (server answers SubmitJob / DescribeJob / AllocStatus / NodeList), Slice 4 (ReconcilerRuntime answers ClusterStatus).

## Outcome

The `overdrive` CLI stub is replaced with real handlers that round-trip through the local control plane over REST + JSON. Operators can submit a job, inspect cluster state, and check allocations. Empty observation tables render explicit empty states that name the next feature rather than silently showing blank output.

## Value hypothesis

*If* the CLI silently hides zero-row observation or computes the spec digest differently from the server, *then* operators lose trust in the platform's honesty long before the platform has anything to lie about. Material honesty from day one is cheaper than retrofitting it after an operator incident traced back to a silent empty table.

## Scope (in)

- `overdrive job submit <spec>`:
  - Reads a TOML file from disk into a `Job` aggregate via serde → rkyv-archive canonical path
  - Opens an HTTP/2 client (rustls-backed) to the configured endpoint — hand-rolled `reqwest`-style or OpenAPI-generated Rust client per DESIGN's pick
  - POSTs `Json<SubmitJobRequest>` to `/v1/jobs`
  - Prints `Job ID`, `Intent key`, `Commit index`, `Endpoint`, and a "Next: …" line
- `overdrive node list`:
  - GETs `/v1/nodes`
  - Renders zero rows as an explicit empty state: "No nodes registered yet — node agent lands in phase-1-first-workload"
  - Renders non-zero rows as a table (with columns: Node ID, Region, Last heartbeat)
- `overdrive alloc status --job <JOB_ID>` or `--alloc <ALLOC_ID>`:
  - GETs `/v1/jobs/{id}` then `/v1/allocs?job={id}`
  - Renders the spec digest, commit index, replicas requested, and the (possibly empty) alloc list
  - Empty state: "No alloc_status rows for this job (nothing is running yet)"
- `overdrive cluster status`:
  - GETs `/v1/cluster/info`
  - Renders mode, region, commit_index, reconciler registry, broker counters
  - Includes the "Phase 1 ships the primitive; job-lifecycle reconciler lands in phase-1-first-workload" footer note
- Actionable error rendering:
  - Connection refused / reset → names the endpoint, suggests `overdrive cluster status` after checking the server is running
  - Validation error (`400 Bad Request` with a structured JSON error body) → surfaces the offending field verbatim
  - NotFound (`404 Not Found`) → names the resource the server did not find
  - Server internal error (`500 Internal Server Error` with a structured JSON error body) → prints the status code and the embedded message, not a raw stack trace
- Exit codes: 0 success, 1 generic error, 2 usage error (matches phase-1-foundation CLI pattern)
- First output within 100ms on localhost

## Scope (out)

- `overdrive job stop` — phase-1-first-workload (needs the job-lifecycle reconciler)
- `overdrive cluster upgrade --mode ha` — Phase 2+ (needs RaftStore)
- `--json` output flag — deferred to Phase 2 when the OpenAPI schema is stable enough to publish for broad SDK consumption
- Shell completion scripts — deferred
- Did-you-mean suggestions — deferred
- Interactive prompts — the CLI stays strictly non-interactive for CI/CD usability per clig.dev

## Target KPI

- Round-trip acceptance test: `overdrive job submit <file>` then `overdrive alloc status --job <id>` prints a spec digest byte-identical to what the input file produces under the same rkyv canonical path
- Every error path prints output answering "what happened / why / what to do next"
- Zero false-cheerful empty states — every empty observation renders a human-readable explanation
- Exit codes match the convention table (0, 1, 2)

## Acceptance flavour

See US-05 scenarios. Focus: honest empty states, spec digest continuity between CLI local compute and server echo, actionable error messages.

## Failure modes to defend

- `alloc status` shows a blank table for zero rows instead of the explicit empty state
- `job submit` hides a validation error behind a generic "failed" message
- The CLI computes spec_digest with a different canonical path (serde_json instead of rkyv) and the digest displayed to the operator doesn't match what the server stored
- Connection refused renders as a raw `ECONNREFUSED` token or raw `reqwest::Error` debug format instead of the actionable message
- The CLI hand-rolls request / response types that shadow the OpenAPI-schema-aligned ones, causing field drift from the server
- Endpoint flag vs env vs default resolution order is undocumented or inconsistent
