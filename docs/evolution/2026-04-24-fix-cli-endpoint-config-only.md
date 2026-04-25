# fix-cli-endpoint-config-only — Feature Evolution

**Feature ID**: fix-cli-endpoint-config-only
**Type**: Bug fix (retroactively archived)
**Branch**: `marcus-sa/phase-1-control-plane-core`
**Date**: 2026-04-24
**Commits**:
- `4d89b8f` — `fix(cli): read endpoint from config, remove --endpoint/OVERDRIVE_ENDPOINT`
- `7227567` — `docs(cli): record config-is-sole-source endpoint rule`
**Status**: Delivered (archive written retroactively on 2026-04-25 — fix shipped without finalize step at the time)

---

## What shipped

The operator CLI now reads the control-plane endpoint exclusively from
`~/.overdrive/config`. The `--endpoint` flag and the `OVERDRIVE_ENDPOINT`
environment variable are removed entirely. There is no override surface;
`cluster init` writes the trust triple (including `endpoint`), and every
subsequent CLI invocation reads from that file as the sole source of truth.

## Bug

`overdrive job submit ./payments.toml` failed with:

```
Error: could not reach the control plane at http://127.0.0.1:7001/.
Cause: transport error.
```

The operator config had `endpoint = "https://127.0.0.1:7001"`. The server
was listening on `https://127.0.0.1:7001`. The CLI was reaching for
`http://127.0.0.1:7001/` — wrong scheme — proving the config was never
consulted.

## Root cause

The CLI's `--endpoint` flag carried an unconditional clap
`default_value = "http://127.0.0.1:7001"` at `crates/overdrive-cli/src/cli.rs:21`.
Because `endpoint: String` had no "unset" state once a default fired, every
handler passed `Some(args.endpoint.as_str())` to
`ApiClient::from_config_with_endpoint`, which short-circuited the config-file
fallback at `crates/overdrive-cli/src/http_client.rs:132`. The
`unwrap_or_else(|| triple.endpoint())` fallback was unreachable in production.

The `http://` scheme in the user-visible error matched the clap default
verbatim — smoking gun.

### Contributing factor

No integration test exercised endpoint resolution through the clap-to-handler
seam. All existing tests passed `endpoint: Url` explicitly because they bound
ephemeral ports.

## Key decisions

- **Remove the override surface entirely.** A flag/env override "defeats the
  purpose of the config" (user rationale at the `/nw-bugfix` Phase 2 review
  gate). Single-cut greenfield migration per the
  `feedback_single_cut_greenfield_migrations` memory: both `--endpoint` and
  `OVERDRIVE_ENDPOINT` deleted in the same commit, no compat shim, no
  deprecation path.
- **Config file is the sole source of truth.** Matches whitepaper §8
  *Operator Identity and CLI Authentication* — the trust bundle, SVID, and
  context all live at `~/.overdrive/config`; the endpoint belongs in the same
  unit of identity, not as a separable override.
- **No new ADR.** The whitepaper already names `~/.overdrive/config` as the
  canonical operator location. This was a conformance fix.

## Regression invariant locked in

`crates/overdrive-cli/tests/integration/endpoint_from_config.rs` — binds an
in-process TLS server on an ephemeral port, writes the operator config
pointing at the server's resolved URL, invokes the `job submit` handler with
no endpoint argument (the field no longer exists), and asserts the POST
reaches the server. The test fails against pre-fix HEAD because the
`SubmitArgs.endpoint` field forced an override; it passes against the fix
because `from_config` reads `triple.endpoint()` as the only source.

## Files affected

| Crate | Files | Change |
|---|---|---|
| `overdrive-cli` | `src/cli.rs`, `src/main.rs`, `src/commands/{job,cluster,alloc,node}.rs`, `src/http_client.rs`, `src/render.rs` | Drop `endpoint` field from `Cli`; delete `parse_cli_endpoint`; remove `endpoint` from each handler's args; collapse `from_config_with_endpoint` into `from_config`; render endpoint from `client.base_url()` instead of `args.endpoint`. |
| `overdrive-cli` | `tests/integration/{job_submit,cluster_and_node_commands,cluster_init_serve,http_client}.rs` | Mechanical updates removing `endpoint` from arg constructions; delete tests that only existed to cover the deleted parameter. |
| `overdrive-cli` | `tests/integration/endpoint_from_config.rs` (new) | Regression test pinning the config-as-sole-source invariant. |
| `overdrive-cli` | `CLAUDE.md` (in `7227567`) | Records the config-is-sole-source rule so a future contributor doesn't reintroduce an override surface. |

## Process notes (retroactive)

This bugfix shipped via `/nw-bugfix` but the DELIVER wave's Phase 7 finalize
step was not run at the time. The RCA at
`docs/feature/fix-cli-endpoint-config-only/deliver/rca.md` was preserved
untracked until 2026-04-25, when this evolution archive was written
retroactively to capture the historical record alongside the
`fix-overdrive-config-path-doubled` and `fix-cli-cannot-reach-control-plane`
archives. No code changed during finalization; only the RCA + this archive
were committed.

## Lineage

This bugfix was followed in close sequence by two further CLI/control-plane
fixes that hardened the same operator-auth flow:

- `fix-overdrive-config-path-doubled` (2026-04-24) — `cluster init` was
  writing to `$HOME/.overdrive/.overdrive/config` (doubled segment); read and
  write paths unified through a single helper.
- `fix-cli-cannot-reach-control-plane` (2026-04-25) — `serve` was writing the
  trust triple to `<data_dir>/.overdrive/config` instead of the operator
  config dir; reqwest classifier rendered TLS handshake failures as "could
  not connect to server".

Together the three bugfixes resolve the operator's documented workflow
(`cluster init` → `serve` → `job submit`) end-to-end with structural
guarantees that the read- and write-sites cannot drift again.
