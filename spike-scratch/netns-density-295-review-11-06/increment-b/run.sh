#!/usr/bin/env bash
set -euo pipefail

# Retained remediation reproduction only. Never invoked by CI, hooks or xtask setup.
probe_rel=spike-scratch/netns-density-295-review-11-06/increment-b
probe_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(CDPATH= cd -- "${probe_dir}/../../.." && pwd)
cd "${repo_dir}"

usage() {
  printf '%s\n' "Usage: bash ${probe_rel}/run.sh {default-kit|loopback-recycle|double-diag-preload}" >&2
  exit 64
}

if [[ ${1:-} == --on-metal ]]; then
  [[ $# == 3 ]] || usage
  probe_case=$2
  capture_rel=$3
  case ${probe_case} in
    default-kit) flags=() ;;
    loopback-recycle) flags=(with-loopback) ;;
    double-diag-preload) flags=(with-loopback with-diag) ;;
    *) usage ;;
  esac
  mkdir -p "${probe_dir}/target" "${repo_dir}/${capture_rel}/guest-console"
  rustc --edition=2024 --target x86_64-unknown-linux-musl \
    "${probe_dir}/src/guest.rs" -o "${probe_dir}/target/review-guest"
  export OVERDRIVE_REVIEW_EVIDENCE_DIR="${repo_dir}/${capture_rel}/guest-console"
  export CARGO_TARGET_DIR="${repo_dir}/target"
  exec cargo run --manifest-path "${probe_dir}/Cargo.toml" --bin review-host -- \
    "${probe_dir}/target/review-guest" "${flags[@]}"
fi

[[ $# == 1 ]] || usage
probe_case=$1
case ${probe_case} in
  default-kit|loopback-recycle|double-diag-preload) ;;
  *) usage ;;
esac
capture_rel="${probe_rel}/evidence/reruns/${probe_case}-$(date -u +%Y%m%dT%H%M%SZ)-$$"
mkdir -p "${capture_rel}"
capture_tmp=$(mktemp "${TMPDIR:-/tmp}/overdrive-review-11-06-b.XXXXXX")
trap 'rm -f "${capture_tmp}"' EXIT
set +e
cargo xtask metal run -- bash "${probe_rel}/run.sh" \
  --on-metal "${probe_case}" "${capture_rel}" 2>&1 | tee "${capture_tmp}"
runner_status=${PIPESTATUS[0]}
set -e
mv "${capture_tmp}" "${capture_rel}/runner.raw.log"
printf '%s\n' "${runner_status}" > "${capture_rel}/runner-exit-status.txt"
printf 'Capture: %s\nRunner exit status: %s\n' "${capture_rel}" "${runner_status}"
exit "${runner_status}"
