# RCA Context — fix-job-submit-body-decode-variant

## Defect

`crates/overdrive-cli/src/commands/job.rs:110-113` maps a server-response
field-validation failure (`JobId::new(&resp.job_id)` rejecting the
returned id) to `CliError::InvalidSpec`. The rendered Display reads:

> `invalid job spec: field 'id': server returned invalid job_id ...`

This blames the operator's spec when the fault is a **server-side
contract violation** — the spec was already accepted; we're decoding
the response.

## Root cause

Variant taxonomy is documented but not honoured at this call site:

- `CliError::InvalidSpec` — `http_client.rs:78-87` — *"Client-side spec
  validation failed BEFORE any HTTP call. … Separate variant from
  `CliError::HttpStatus` — the client-side path never reaches the
  server."*
- `CliError::BodyDecode` — `http_client.rs:75-76` — *"A successful 2xx
  response whose body failed to deserialise into the expected typed
  shape. This is a server-side contract violation."*

The call site at `job.rs:110-113` is post-HTTP. A `JobId` validator
failure on `resp.job_id` is semantically identical to the
`resp.json::<T>()` decode failure at `http_client.rs:242`, which is
already mapped to `BodyDecode`.

## Fix

Swap the variant at `job.rs:110-113`:

```rust
// before
let job_id = JobId::new(&resp.job_id).map_err(|e| CliError::InvalidSpec {
    field: "id".to_string(),
    message: format!("server returned invalid job_id `{}`: {e}", resp.job_id),
})?;

// after
let job_id = JobId::new(&resp.job_id).map_err(|e| CliError::BodyDecode {
    cause: format!("server returned invalid job_id `{}`: {e}", resp.job_id),
})?;
```

Operator-facing rendering changes from:

> `invalid job spec: field 'id': server returned invalid job_id ...`

to:

> `failed to decode response body from control plane: server returned invalid job_id ...`

## Files affected

- `crates/overdrive-cli/src/commands/job.rs` — single error mapping
  (4 lines).
- `crates/overdrive-cli/tests/integration/<new>.rs` — regression test
  asserting the variant against a stub control plane that returns a
  syntactically-valid JSON body whose `job_id` fails `JobId::new()`.

## Regression-test contract

GIVEN a stub control plane returning `200 OK` with
`SubmitJobResponse { job_id: "<invalid>", commit_index: 1 }` where
`<invalid>` is JSON-decodable but fails `JobId::new()`,
WHEN `commands::job::submit(args)` is invoked,
THEN the returned `CliError` is `BodyDecode { cause }` with `cause`
containing `"server returned invalid job_id"`,
AND is NOT `InvalidSpec { .. }` (variant choice is pinned).

The test lives behind the `integration-tests` feature gate per
`.claude/rules/testing.md`. Per `crates/overdrive-cli/CLAUDE.md`, the
test calls `submit()` directly — no subprocess.

## Risk

Low. Pure variant remap; no signature, type, or transport change. The
`submit()` rustdoc (`job.rs:69-77`) already documents `BodyDecode` as
"the 2xx response body failed to parse" — the fix aligns the code with
the contract.
