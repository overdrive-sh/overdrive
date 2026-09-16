#!/usr/bin/env bash
# E10 cleanup oracle was tied to the pre-cut Exec/VM comparison matrix.
# Historical evidence remains immutable; current VM-only cleanup is covered by
# the retained E08/E09/E11-E13 journeys.
set -euo pipefail
echo 'E10 cleanup oracle retired pre-cut: VM-only evidence is retained separately.' >&2
exit 2
