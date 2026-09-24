# System Conformance Test Rules

These rules apply to everything under `tests/conformance/` and specialize
`.claude/rules/testing.md` § "Root-level system conformance."

## Driving boundary

- For server/API contracts, start the exported production server handler
  directly inside the Rust test using the production composition path.
- Drive behavior through the server's public API. Do not route API-owned
  behavior through `overdrive deploy`, CLI command handlers, stdout, or config
  discovery.
- Never spawn the `overdrive` binary or drive the CLI. The conformance harness
  starts the server in-process and uses its public API only. Running the built
  CLI is reserved for a verification expectation that verifies the CLI's own
  output (`.claude/rules/testing.md` § "Classify external execution before
  writing it").
- Never call internal application components as the behavior-driving port.
  Public constructors and accepted injected production-owner helpers may be
  used only to build the real composition or inject an approved driven-port
  fault.

## Reusable harness

- Shared server composition, API client, trust material, bounded waiting,
  lifecycle/restart, host setup, fault injection, observation, and cleanup
  helpers live in `src/lib.rs`.
- Scenario files under `tests/` contain only scenario-specific Given/When/Then
  setup, stimulus, and assertions. Do not copy a `spawn_server` helper into each
  test file.
- Starting a replacement server means constructing the exported handler again
  against the retained production data directory. It does not mean invoking
  the operator CLI or manufacturing persisted state.

## Host and kernel fixtures

- Prefer typed Rust control surfaces. Use `nix` for direct OS primitives and
  `overdrive-netlink` for link, TAP, route, nftables, and read-back operations
  it owns.
- Do not shell out to `ip` as the default setup, mutation, or cleanup
  mechanism. External tools are reserved for an independent kernel oracle or a
  surface with no typed project/library API.
- Faults are introduced through accepted driven ports or real host state. Do
  not seed a derived failure consequence or add a test-only production API.
- Cloud Hypervisor/KVM and other real-guest evidence runs only on qualified,
  non-virtualized x86_64 native metal through `cargo xtask metal run --`.

## Assertions and ownership

- Assert public API responses, exported handler lifecycle results, and
  observable host/kernel effects. Do not assert private fields merely because
  the conformance crate can depend on production crates.
- Preserve the distinction between an in-process system conformance test and a
  crate-local integration test: this suite owns only contracts spanning
  multiple production owners with no natural single-crate home.
- Keep Rust integration assertions, EDD expectations, and benchmark receipts
  separate. Conformance tests are recurring and deterministic; expectations
  are SHA-pinned one-time evidence; benchmarks measure distributions.
- Every host mutation has an exact cleanup complement. Cleanup failures retain
  the original failure and the later cleanup evidence rather than overwriting
  either.

## Execution

- Use `cargo nextest run`, never `cargo test`, for the Rust conformance suite.
- Route execution through Lima for ordinary Linux conformance and through the
  native-metal runner for tests marked as requiring KVM or real guest/kernel
  behavior.
- Native-metal tests remain reasoned-ignored in the ordinary lane and are
  activated only by the dedicated `tests/conformance/run-native-metal.sh`
  entrypoint.
