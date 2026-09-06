#!/usr/bin/env bash
set -euo pipefail

readonly EXAMPLE="$REPO_ROOT/examples/service-kind-vm-workloads/run-example.sh"
"$EXAMPLE" check-source
"$EXAMPLE" run zero-probes
