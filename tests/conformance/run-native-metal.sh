#!/usr/bin/env bash
# Native-metal entrypoint reserved for reasoned-ignored KVM conformance cases.
# Direct-handler/API recovery tests without a real guest run under ordinary
# nextest and are deliberately excluded from this selector.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly repo_root
readonly manifest="$repo_root/tests/conformance/Cargo.toml"

die() { echo "system conformance: $*" >&2; exit 1; }
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || die "native x86_64 Linux required"
[[ "$(id -u)" -eq 0 ]] || die "run through cargo xtask metal run --"
[[ "$(systemd-detect-virt 2>/dev/null || true)" == none ]] || die "virtualized substrate refused"
[[ -c /dev/kvm ]] || die "usable /dev/kvm required"

exec cargo nextest run --manifest-path "$manifest" --features integration-tests \
  --run-ignored ignored-only --no-capture -E 'test(/^native_metal_kvm_/)'
