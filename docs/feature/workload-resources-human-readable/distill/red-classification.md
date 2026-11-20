# DISTILL RED classification: workload-resources-human-readable (GH #372)

Every pending test is `#[should_panic(expected = ...)]`, so it passes while the behaviour is missing and fails the day the behaviour exists. The `expected` substring is the semantic RED. All pending tests are **MISSING_FUNCTIONALITY**: each reaches the assertion or decode it was written to fail on. None is IMPORT_ERROR, FIXTURE_BROKEN, SETUP_FAILURE, WRONG_ASSERTION or OBSERVABLE_NOT_AT_PORT.

## Classification

| Group (crates/) | Tests | Class | Semantic RED (`expected`) |
|---|---|---|---|
| `overdrive-core` `byte_size_grammar`: accept table, rejection classes and order, error text, serde bare number, proptests | 14 | MISSING_FUNCTIONALITY | the scaffold refuses every input and renders nothing: `the binary byte-size grammar must accept the pinned input`, `the grammar must classify`, `the error text must name`, `the expected string form`, `a byte size must display canonically`, `the u64 boundary must hold`, ... |
| `overdrive-core` `byte_size_grammar`: volume refinement | 1 | MISSING_FUNCTIONALITY | `the volume size must accept the pinned input` (the shipped `VolumeSize` parser refuses `1024MiB`: its unit must be `GiB` or `TiB`; `40GiB` and `2TiB` already pass) |
| `overdrive-core` `cpu_quantity_grammar` | 12 | MISSING_FUNCTIONALITY | same shape: `the cpu grammar must accept the pinned input`, `the cpu grammar must classify`, `the error text must`, `a cpu quantity must display canonically`, `the u32 boundary must hold`, ... |
| `overdrive-core` `workload_resources_spec`: `a_non_canonical_spelling_is_read_back_canonically` | 1 | MISSING_FUNCTIONALITY | `a spec in the new resources shape must parse`: the TOML parser still says `unknown field cpu`. This is the one test whose RED is the TOML parser itself |
| `overdrive-core` `workload_resources_spec`: removed keys, missing key, malformed memory, malformed cpu | 4 | MISSING_FUNCTIONALITY | `the removed key must be refused at parse time`, `the missing key must be named`, `the refusal must not point the operator at a removed key` (today's refusal names `cpu_milli` / `memory_bytes` as the expected keys) |
| `overdrive-core` `workload_resources_spec`: wire removed keys | 1 | MISSING_FUNCTIONALITY | `the wire must refuse the removed key naming it` |
| `overdrive-core` `workload_resources_projections`, `workload_resources_spec`: carries-size, admission matrix, digest, describe (named in § "Tests whose RED is the string wire") | 5 | MISSING_FUNCTIONALITY | `string cpu/memory resources are not accepted yet`: the string wire does not decode. **Unreachable until the string wire exists**; see the note below |
| `overdrive-core` `workload_resources_examples` | 1 | MISSING_FUNCTIONALITY | `every example spec must parse with the human-readable resources` (the 33 example specs still carry `cpu_milli`) |
| `overdrive-core` `schema_evolution::workload_resources_fixtures` | 2 | MISSING_FUNCTIONALITY | `the archived resources must read back as canonical strings`; the fixture bytes already decode through the current envelope, only the read-back differs |
| `overdrive-cli` `deploy_workload_resources`: removed keys, accepted control | 2 | MISSING_FUNCTIONALITY | `the spec must be refused as a spec error before any server call` (a removed key reaches the config stage); `an accepted spec must pass local validation` (`unknown field cpu`) |
| `overdrive-cli` `deploy_workload_resources`: malformed memory, malformed cpu | 2 | MISSING_FUNCTIONALITY | `the refusal must not point the operator at a removed key` |
| `overdrive-cli` `deploy_workload_resources`: zero and sub-MiB memory | 1 | MISSING_FUNCTIONALITY | `the refusal must name the memory field` (the refusal's field is `toml`) |
| `overdrive-cli` `render_workload_resources` (named in § "Tests whose RED is the string wire") | 4 | MISSING_FUNCTIONALITY | `string cpu/memory resources are not accepted yet`: the describe wire does not decode strings. **Unreachable until the string wire exists** |
| `overdrive-control-plane` `workload_resources_wire`: emitted canonical strings | 1 | MISSING_FUNCTIONALITY | `the resources block with string cpu and memory must decode` |
| `overdrive-control-plane` `workload_resources_wire`: removed keys | 1 | MISSING_FUNCTIONALITY | `the removed keys must not decode` |
| `overdrive-control-plane` `workload_resources_wire`: OpenAPI schemas | 1 | MISSING_FUNCTIONALITY | `the OpenAPI schema must describe cpu and memory as strings` |
| `overdrive-control-plane` `workload_resources_wire`: submit handler refuses sub-MiB memory | 1 | MISSING_FUNCTIONALITY | `the submit must be refused as a 400 validation naming the memory field`: today the extractor answers 422 to the string body. **Unreachable until the string wire exists** |

## Tests whose RED is the string wire, and what DELIVER does

Nine tests stop at the helper that decodes the new string shape and fail with `string cpu/memory resources are not accepted yet`:

- `overdrive-core` `workload_resources_spec`: `an_admitted_workload_carries_the_declared_size_in_its_intent_resources`, `sad_admission_follows_the_non_zero_and_whole_mib_rules_at_both_sites`, `equal_sizes_in_different_spellings_have_the_same_spec_digest`, `describe_reads_back_the_canonical_form`
- `overdrive-core` `workload_resources_projections`: `no_capacity_prose_renders_the_canonical_quantities`
- `overdrive-cli` `render_workload_resources`: `describe_prints_the_canonical_memory_and_cpu_lines_for_every_kind`, `describe_prints_the_size_once_for_a_multi_replica_workload`, `describe_prints_a_zero_cpu_as_the_canonical_zero`, `describe_omits_both_size_lines_without_a_resolved_size`

The assertions after the decode helper (the canonical read-back, the whole-MiB and zero-memory refusals, the digest equality, the describe lines) are not exercised by the RED run.

One further test is unreachable past the string wire for a different reason: `overdrive-control-plane` `workload_resources_submit_refusal::sad_memory_that_is_not_a_whole_number_of_mib_is_refused_and_nothing_is_stored` (the submit handler). Its RED is the 422 the JSON extractor answers to the string body, so the 400 validation naming the memory field is not reached.

All ten bodies are complete. DELIVER removes each marker in the rename step that makes the string wire decode, runs the body unchanged, and confirms the next semantic RED, if any, before changing production code. No rename-independent probe was added: a probe would need a body edit after the rename, which DELIVER must not make.

`a_non_canonical_spelling_is_read_back_canonically` is the one test whose RED is the string wire at the TOML parser itself.

## Not pending-RED

- **Compiled out (0):** none. `node_capacity_resources.rs` was deleted (AC-1.10 coverage is compiler-enforced field types plus the existing `aggregate_validation.rs`, see the feature delta § AC-1.10 coverage).
- **Not run on this substrate (0):** the guest-placed round-trip file was replaced by the host-pure submit-handler test in `workload_resources_wire`; every pending test runs in the default lane.

## Observation for DELIVER (not a failure)

For a `[job]` spec, the deploy handler's flat-spec fall-through already surfaces the nested unknown field in its message today (`unknown field cpu, expected cpu_milli or memory_bytes`). After the rename the same path must keep naming the removed key and the expected `cpu` / `memory` for all three spec shapes; `sad_deploy_refuses_the_removed_keys_naming_the_unknown_key` asserts exactly that and accepts either `CliError::InvalidSpec` or `CliError::ParseError`.
