# ADR-0304 — The Metal benchmark form runs a closed command under exclusive admission

## Status

**Accepted.** Approved by the user 2026-11-17; independent design review resolved 2026-11-17. DDD-9 in the feature delta.

## Context

Ruling 9: add a benchmark form to `cargo xtask metal run`, run benchmarks exclusive,
and give `testing.md`'s "no test is ever run with `--exclusive`" rule a benchmark
exemption. An `--exclusive` run today skips command validation and accepts any
command. The ordinary grammar does not accept `cargo bench`. `metal pull` retrieves
repo-relative paths outside `target/`.

## Decision

`cargo xtask metal run --benchmark <FAMILY> [--spike <NAME>]
[--no-sync] [--no-sudo]` is the benchmark form. It is exclusive by construction
(it neither needs nor accepts `--exclusive` or a trailing command) and has no seed
option: the runner's seed comes from OS entropy and is recorded. `bootstrap.sh`
gains a `--benchmark` mode that validates `<FAMILY>` and `<NAME>` against
closed patterns and builds the remote command itself (`$ROOT` is the exported
`OVERDRIVE_METAL_SOURCE_ROOT`):

- `cargo bench -p overdrive-bench-families --bench benchmarks -- run --family <FAMILY> --results-root $ROOT/benchmarks/results`
- with `--spike <NAME>`: `cargo bench --manifest-path spike-scratch/<NAME>/Cargo.toml --bench benchmarks -- run --family <FAMILY> --results-root $ROOT/spike-scratch/<NAME>/results`

`cargo bench` runs with the package directory as its working directory, so the
results root is an absolute path under `$ROOT`; the runner refuses a relative one.
The run is admitted with execution class `exclusive`, so the wrapper does not
apply placement. The runner prints `OVERDRIVE_BENCH_RESULT_DIR=<repo-relative
path>` (relative to `$ROOT`), and `cargo xtask metal pull <path>` retrieves it. The orchestrator amends
`testing.md`: the exclusive rule exempts benchmarks beside expectation captures,
and the command-grammar section names the benchmark form.

## Alternatives

- **`--exclusive -- cargo bench ...`**: runs an unvalidated command; the closed
  form removes that; rejected.
- **A separate `metal bench` subcommand**: ruling 9 says a form of `metal run`;
  rejected.
- **Accept `cargo bench` in the ordinary grammar**: ordinary runs overlap, which
  invalidates a benchmark; rejected.

## Consequences

Prerequisites (`bpf-build`, `guest-kit-prepare`, `kmod-load`) stay separate
commands, as today. Edits land in `xtask/src/main.rs` (clap) and
`infra/metal/bootstrap.sh`.

## Links

- [Decision DDD-9 and the command form](../../feature/native-benchmark-program/feature-delta.md#ddd-9--the-metal-benchmark-form)
