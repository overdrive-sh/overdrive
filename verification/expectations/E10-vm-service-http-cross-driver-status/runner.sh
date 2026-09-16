#!/usr/bin/env bash
# E10 was a pre-cut Exec/VM comparison expectation. The current supported
# execution path is VM/microVM-only; VM HTTP/TCP evidence remains in E08/E09/E11-E13.
# Historical E10 evidence remains immutable.
set -euo pipefail
echo "E10 retired pre-cut: cross-driver fixture removed; VM-only evidence is retained separately." >&2
exit 2
