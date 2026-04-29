# Suggested Commands for Overdrive Development

## Compile/check (NEVER use `cargo build` — blocked by hook)
```bash
cargo check                          # fast typecheck
cargo check -p overdrive-core        # single crate
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets
```

## Test (NEVER use `cargo test` except doctests — blocked by hook)
```bash
cargo nextest run                    # all unit tests
cargo nextest run -p <crate>         # single crate
cargo nextest run -p <crate> -E 'test(<filter>)'
cargo test --doc -p <crate>          # doctests only
cargo nextest run --features integration-tests  # slow integration tests
```

## DST / xtask
```bash
cargo xtask dst                      # Tier 1: turmoil DST
cargo xtask bpf-unit                 # Tier 2: BPF unit tests
cargo xtask integration-test vm      # Tier 3: real-kernel tests
cargo xtask verifier-regress         # Tier 4: veristat
cargo xtask xdp-perf                 # Tier 4: xdp-bench
cargo xtask dst-lint                 # dst-lint for core crate violations
cargo xtask mutants --diff origin/main --features integration-tests
cargo xtask openapi-gen              # generate api/openapi.yaml
cargo xtask openapi-check            # check drift in openapi.yaml
```

## Git
```bash
git diff origin/main...              # diff against main
git log --oneline -10
```
