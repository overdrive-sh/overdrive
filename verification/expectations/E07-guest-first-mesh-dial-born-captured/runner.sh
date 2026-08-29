#!/usr/bin/env bash
# shellcheck shell=bash
# Pending black-box E07 runner. It will drive the checked-in example with the
# built default-feature product; D7/TLS/kernel internals remain Rust-test owned.
set -uo pipefail

example_dir="$REPO_ROOT/examples/guest-stack-transparent-mtls-intercept"
required=(caller.toml callee.toml caller.rs callee.rs)
for fixture in "${required[@]}"; do
  [[ -f "$example_dir/$fixture" ]] || {
    echo "missing checked-in E07 fixture: $example_dir/$fixture" >&2
    exit 1
  }
done

echo "  [pending] E07 needs the roadmap-regenerated native-metal runner."
echo "            It will build/materialize the checked-in example and drive only"
echo "            built serve/deploy/describe/stop product commands."
exit 0
