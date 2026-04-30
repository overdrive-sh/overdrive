# Slice 3 — `--detach` flag and pipe auto-detection (CONDITIONAL)

**Feature**: `cli-submit-vs-deploy-and-alloc-status`
**Wave**: DISCUSS / Phase 2.5
**Owner**: Luna
**Status**: CONDITIONAL — only carved out if Slice 2's complexity
budget is at risk. Otherwise folded into Slice 2.

## Goal

Pull `--detach` and `isatty(stdout)`-based auto-detection out of
Slice 2 if Slice 2's implementation surface (server NDJSON emitter +
CLI consumer + exit-code mapping + regression-target test) starts
threatening the ≤1-day budget.

## IN scope (when activated)

- `--detach` flag on `overdrive job submit`. When present, CLI sends
  `Accept: application/json` regardless of TTY. Exits 0 immediately
  on the JSON ack.
- `isatty(stdout)` detection. When stdout is NOT a TTY, CLI sends
  `Accept: application/json` (auto-detach) so pipelines like
  `submit | jq` work without breaking.
- AC asserting that `submit | jq -r .spec_digest` outputs the digest
  to stdout and exits 0.

## OUT scope

- All NDJSON streaming machinery — that's Slice 2.
- `--quiet`, `--no-stream`, or any other flag.
- Auto-`--detach` based on environment variables (e.g.
  `CI=true`) — explicit `isatty` is the only signal.

## Learning hypothesis

**Disproves**: auto-detach via TTY detection if CI environments
allocate TTYs that confuse the heuristic, OR if operators end up
passing `--detach` defensively even in pipelines where the heuristic
would have fired. Either signal means TTY detection is the wrong
shape and the explicit flag alone should be the contract.

## Acceptance criteria

1. `overdrive job submit ./job.toml --detach` interactively sends
   `Accept: application/json`, prints one JSON line, exits 0.
2. `overdrive job submit ./job.toml | jq -r .spec_digest` works:
   stdout is the digest, exit 0.
3. `overdrive job submit ./job.toml > /tmp/out.json` (redirected)
   produces a single JSON object in the file, exits 0.
4. Without `--detach` and with stdout as a TTY, the CLI streams (the
   default Slice-2 behaviour).

## Dependencies

- Slice 2 (cannot decouple `--detach` from the streaming default).

## Effort estimate

≤0.5 day.

## Activation criterion

DESIGN flags Slice 2 as approaching the ≤1-day budget DURING Slice 2
implementation, and proposes splitting `--detach` + auto-detach as a
follow-up cut. If Slice 2 lands cleanly with `--detach` and TTY
detection inline, this slice is closed without further work. The
brief stays in tree as a record of the conditional split.

## Reference class

`docker run` (`-d`), `nomad job run` (`--detach`), `kubectl apply`
(no equivalent — but the operator's mental model around `-d`-style
flags is uniform across the ecosystem).
