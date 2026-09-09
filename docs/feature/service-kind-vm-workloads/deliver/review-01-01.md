# DELIVER review — 01-01 Atomic ServiceSpecV3 ingress

## Metadata

| Field | Value |
|---|---|
| Feature | `service-kind-vm-workloads` |
| Step | `01-01` — Atomic ServiceSpecV3 ingress |
| Reviewer | Independent DELIVER reviewer |
| Reviewed commits | `49d3ffa0`, `9250df90`, `de34682c` |
| Final accepted contract | User-authorized greenfield amendment in ADR-0091; updated feature design and roadmap step 01-01 |
| Final verdict | **APPROVED** |

## Final contract

The user-authorized greenfield amendment supersedes the earlier boxed-V3
compatibility contract. The sole parser/persistence envelope arm is exactly
`V3(ServiceSpecV3)` at tag `[0]`; public `ServiceSpec` and
`ServiceSpecLatest` are direct unboxed V3 aliases. V1/V2 types, arms,
fixtures, conversion/migration/re-archive paths, compatibility fields,
accessors, and decoders must be absent.

The existing parser/wire/intent driver unions remain the only Service-driver
model. VM Services with HTTP/TCP probes are admitted, while both ingress
boundaries reject the first VM Exec probe in Startup → Readiness → Liveness
then vector-position order, using the exact GH #280 diagnostic and existing
localized error surfaces. The two existing CLI deploy lanes must preserve the
selected driver and existing response/render semantics.

## Iteration 1 — `49d3ffa0`

### Verdict

**NEEDS_REVISION**

### R1 — CLI acceptance scenarios did not exercise either CLI lane

| Field | Evidence |
|---|---|
| Severity | High — test honesty / required driving-port evidence |
| Test gap | The initial S-SVM-23/24 tests only called `WorkloadSpecInput::from_toml_str` and asserted `Service(_)`. |
| Reachable production paths | Detached: public `deploy` → `deploy_service` → driver projection → `SubmitSpecInput::Service`. Streaming: public `deploy_streaming` → `deploy_streaming_service` → driver projection → existing streaming request. |
| Failure proof | Collapsing VM to Exec, dropping `kernel`/`rootfs`, or selecting a parallel request shape left both tests green because neither reached the CLI lane. |
| Disposition | Remediated in `9250df90`: detached traffic now drives public `deploy` and verifies the committed Service VM arm. |

## Iteration 2 — `9250df90`

### Verdict

**NEEDS_REVISION**

### R2 — streaming render/exit assertion could be skipped

| Field | Evidence |
|---|---|
| Severity | High — required CLI acceptance evidence |
| Test gap | Output assertions were only in the completed-stream match arm; the timeout arm aborted the client and passed. |
| Failure proof | The focused Lima run took 2.620 seconds, matching the test's 500 ms sleep plus 2 s timeout, proving the no-assertion timeout branch executed. |
| Disposition | Remediated in the committed `de34682c` result: public `deploy_streaming` receives a deterministic existing Accepted → Stopped NDJSON stream; it unconditionally asserts workload id, exit code, exact renderer output, and the captured Service VM request arm. |

## Iteration 3 — `de34682c` greenfield re-review

### Verdict

**APPROVED**

### Design and implementation evidence

- `crates/overdrive-core/src/aggregate/service_spec.rs:19-39` aliases both
  public names to direct `ServiceSpecV3` and declares only
  `V3(ServiceSpecV3)`.
- `service_spec.rs:66-80` constructs and projects that arm directly and pins
  the sole known tag to `[0]`; it introduces neither a box nor a fallback
  codec.
- `crates/overdrive-core/tests/schema_evolution/service_spec.rs:40-55`
  defines one direct current V3 fixture, archives through `latest`, decodes
  it to the exact latest payload, and asserts tag zero.
- Repository search found no ServiceSpec V1/V2 type, envelope arm, fixture,
  conversion, compatibility decoder/prefix, or accessor surface. The remaining
  unrelated versioned-envelope fixtures belong to other aggregates.
- Parser VM-Exec rejection remains before `ServiceSpec` construction at
  `workload_spec.rs:861-875`, scanning the three role vectors in the required
  order; its helper selects the first vector position and projects the required
  `ParseError::Field` section/message.
- Direct admission retains the same role-vector selection before reindexing and
  before a `ServiceV2` value can be returned. Describe maps both persisted
  driver arms field-for-field through the existing wire union.
- S-SVM-23 drives the existing public detached lane through the in-process
  server and reads the committed Service intent. S-SVM-24 drives public
  `deploy_streaming`, captures the established `/v1/workloads` Service
  request, validates all VM fields, and unconditionally proves the existing
  Stopped render and exit outcome. The new TLS endpoint is test-only evidence;
  no production request type, endpoint, API, or lifecycle behavior was added.

### Scope and API assessment

The greenfield storage reset is explicitly user-authorized and exactly matches
the amended ADR/roadmap: removing the old ServiceSpec compatibility format is
required, while the live `ServiceV2` intent format is unchanged. The test-only
`axum`/`axum-server` dev dependencies are tightly scoped to truthful
streaming CLI evidence. No new public type, enum variant, method, trait,
parameter, decoder, accessor, compatibility field, or alternate request shape
was introduced.

## Verification

| Command | Result |
|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-core -p overdrive-cli --test acceptance -E 'test(service_kind_vm_workloads)'` | Passed: 10 tests. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-core --test schema_evolution -E 'test(service_spec_v3_direct_fixture_round_trips_with_sole_tag_zero)'` | Passed: 1 test. |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | Passed. |

Mutation testing was not run; it is a final-wave-only gate.

## Final verdict

**APPROVED.** R1 and R2 are resolved in committed work, and the
user-authorized greenfield persistence contract is implemented exactly. No
reachable correctness, API-shape, test-honesty, or scope finding remains for
step 01-01. Unrelated dirty roadmap/design/ADR/log files were preserved.

