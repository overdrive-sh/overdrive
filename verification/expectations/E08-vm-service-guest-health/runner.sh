#!/usr/bin/env bash
# E08 black-box expectation. The checked-in product example owns the exact
# built-binary journey and currently returns 75 as an honest DISTILL pending.
set -euo pipefail

readonly EXAMPLE="$REPO_ROOT/examples/service-kind-vm-workloads/run-example.sh"

"$EXAMPLE" check-source
"$EXAMPLE" run healthy
