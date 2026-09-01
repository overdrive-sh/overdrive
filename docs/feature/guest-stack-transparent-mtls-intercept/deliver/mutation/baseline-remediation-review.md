# Mutation Baseline OpenAPI Remediation Review

## Metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Review scope | Final DELIVER mutation-baseline OpenAPI remediation only |
| Reviewed commit | `42aed6f997847357b70df222bb2e00ee0f86a455` |
| Parent | `eccd3fbc90ddc19d3e083ba7118368dc3860c42c` |
| Range | `eccd3fbc90ddc19d3e083ba7118368dc3860c42c..42aed6f997847357b70df222bb2e00ee0f86a455` |
| Subject | `chore(openapi): regenerate allocation address schema` |
| Required trailer | `Feature-Id: guest-stack-transparent-mtls-intercept` — present and exact |
| Review iteration | 1 |
| Verdict | **APPROVED** |

## Review boundary

This review is limited to contract alignment, generated-artifact honesty, and
regression risk for the OpenAPI baseline remediation. It does not re-review the
endpoint implementation, unrelated OpenAPI/job-streaming surfaces, or the
feature's mutation corpus. No mutation testing was run.

## Source-of-truth comparison

The checked-in property is an exact projection of the source schema at
`crates/overdrive-control-plane/src/api.rs:390-397`.

| Contract element | Rust source of truth | Generated `api/openapi.yaml` | Result |
|---|---|---|---|
| Property name | `workload_addr` | `AllocStatusRowBody.properties.workload_addr` | PASS |
| Wire scalar | `#[schema(value_type = Option<String>)]` | `type: [string, 'null']` | PASS |
| Optionality | `Option<Ipv4Addr>` plus `serde(default, skip_serializing_if = "Option::is_none")` | Nullable and absent from `AllocStatusRowBody.required` | PASS |
| Documentation | Canonical workload endpoint address; VM guest-NIC `/30`; never transit-veth; compatibility rationale | Verbatim source rustdoc | PASS |

The property appears only under `AllocStatusRowBody`, as the generator orders
it between `state` and `workload_id`. The reviewed range does not modify
`api.rs`, any endpoint, DTO, test, generator, or other public API source. The
change therefore repairs generated-schema drift without inventing or altering
API surface.

## Diff scope and artifact honesty

`git diff --name-status` and `git diff --numstat` show exactly one changed file:

- `api/openapi.yaml`: 10 insertions, 0 deletions.

The ten lines add only the nullable string `workload_addr` property and its
source documentation. No path, operation, required-field set, existing
property, component schema, source file, test, or configuration changed.
`git diff --check` passed. The reviewed commit's direct parent is the supplied
parent, and its conventional subject and required `Feature-Id` trailer are
correct.

## Verification

| Command | Result |
|---|---|
| `git rev-parse 42aed6f9^` | PASS — `eccd3fbc90ddc19d3e083ba7118368dc3860c42c` |
| `git diff --name-status` / `--numstat` / `--check` over the reviewed range | PASS — only `api/openapi.yaml`, `10 0`, no whitespace errors |
| `cargo xtask lima run -- cargo openapi-gen` | PASS — official generator completed and produced no tracked diff |
| Repeat `cargo xtask lima run -- cargo openapi-gen` | PASS — identical SHA-256 `b6a55e7e5fd4718735048d350a15ece9c7a9d0b6de518228298960d22b4b3183`; no further diff |
| `cargo xtask lima run -- cargo openapi-check` | PASS — exact checked-in-vs-live gate exited 0 |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --features integration-tests -E 'test(openapi)'` | PASS — 8 passed, 0 failed, 789 skipped by selection |

A preliminary host-native `cargo openapi-gen` attempt failed during dependency
compilation because Linux-only `linux-keyutils`, netlink, and aya APIs are not
available on macOS. It did not execute the generator and left
`api/openapi.yaml` unchanged. The canonical Lima invocations above are the
authoritative repository signal and all passed.

## Findings

No defects found. The generated property matches the source-of-truth type,
optionality, and documentation; the change is tightly scoped; generation is
deterministic; the exact drift gate and selected OpenAPI tests pass; and the
commit metadata is correct.

## Verdict

**APPROVED.** Commit `42aed6f997847357b70df222bb2e00ee0f86a455`
honestly restores the checked-in OpenAPI artifact to the live
`AllocStatusRowBody` schema with no unrelated API change.
