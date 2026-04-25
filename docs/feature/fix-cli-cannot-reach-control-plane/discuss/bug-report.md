# Bug report — CLI cannot reach the control plane it just started

## Defect

An operator running the Phase 1 single-node flow from a clean machine
sees the following:

```
$ cargo run -p overdrive-cli --bin overdrive -- serve
2026-04-24T16:21:09.271345Z  INFO control plane listening \
  endpoint=https://127.0.0.1:7001/

$ cargo run -p overdrive-cli --bin overdrive -- job submit ./payments.toml
Error: could not reach the control plane at https://127.0.0.1:7001/.
Cause: could not connect to server.
```

Both sides name `https://127.0.0.1:7001`. The server process is still
alive. TCP connect to `127.0.0.1:7001` succeeds against the listener.
The CLI still reports `could not connect to server`.

## Impact

The only documented way to bring up Phase 1 single-mode and verify it
end-to-end is:

1. `overdrive serve` in one terminal.
2. `overdrive job submit <spec>` in another.

Step 2 fails on every clean bootstrap. The acceptance flow advertised
by every `*_submit*` integration test passes in CI but no single
operator CLI path reproduces it — the test suite manufactures a
coincidence the production binary cannot.

## Severity

**P0 for Phase 1 acceptance.** This is the walking-skeleton path. A
first-time operator following the README cannot get past step 2.
Secondary impact: every non-CI operator demo is broken by default.

## What is NOT the bug

- The server is running — `axum_server` is live on `127.0.0.1:7001`.
- The endpoint strings match — both sides agree on
  `https://127.0.0.1:7001`.
- The config parses — `load_trust_triple` succeeds (the error is
  `CliError::Transport`, not `CliError::ConfigLoad`).
- It is not a port collision, firewall, or IPv4/IPv6 confusion — `::1`
  and `127.0.0.1` are both SANs on the minted server leaf, and the
  listener binds exactly the address the URL names.

## What IS the bug (summary; full chain in `deliver/rca.md`)

`overdrive serve` writes the freshly-minted trust triple to
`<data_dir>/.overdrive/config`, which on a default invocation resolves
to `$HOME/.local/share/overdrive/.overdrive/config` (XDG data dir per
ADR-0013 §5 plus the `.overdrive/config` suffix `write_trust_triple`
always appends).

`overdrive job submit` reads its trust triple from
`default_operator_config_path()`, which on a default invocation
resolves to `$HOME/.overdrive/config` (whitepaper §8 / ADR-0019
canonical operator config path).

These are two different files. The file the CLI reads was not written
by *this* `serve` invocation — it was a leftover from a prior
`cluster init`. The endpoint happens to match; the CA and leaf certs
do not. `reqwest` pins the stale CA as its sole root of trust, the
TLS handshake fails against the new server leaf, reqwest classifies
the error as `is_connect()`, and the CLI renders it as "could not
connect to server" — the bug that looks like a networking problem is
actually a *trust-material mismatch*, surfaced through a transport
error classifier that lumps handshake failures into the connect
bucket.

## Why the test suite does not catch this

Every integration test under
`crates/overdrive-cli/tests/integration/` and
`crates/overdrive-control-plane/tests/integration/` passes a single
`TempDir` as `data_dir` and also reads from
`tmp.path().join(".overdrive").join("config")`. The write site and
the read site agree by construction because the test supplies both.
Production diverges because `default_config_path()` (read) and
`default_data_dir()` (write) are derived from different env vars with
different suffixes.

Full chain, evidence, and proposed fix live in `../deliver/rca.md`.
