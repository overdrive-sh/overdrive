#!/usr/bin/env bash
set -euo pipefail
if [ "$1" = "--version" ]; then
  printf 'clang version 21.0.0\n'
  exit 0
fi
printf 'clang' >> "${OVERDRIVE_FAKE_COMMAND_LOG}"
printf '\t%s' "$@" >> "${OVERDRIVE_FAKE_COMMAND_LOG}"
printf '\n' >> "${OVERDRIVE_FAKE_COMMAND_LOG}"
out=''
candidate_source='false'
generated_header='false'
libbpf_headers='false'
include_next='false'
target_next='false'
debug='false'
optimized='false'
bpf_target='false'
target_arch='false'
while [ "$#" -gt 0 ]; do
  if [ "${include_next}" = 'true' ]; then
    if [ -f "$1/vmlinux.h" ]; then generated_header='true'; fi
    if [ -f "$1/bpf/bpf_helpers.h" ]; then libbpf_headers='true'; fi
    include_next='false'
  fi
  if [ "${target_next}" = 'true' ]; then
    if [ "$1" = 'bpf' ]; then bpf_target='true'; fi
    target_next='false'
  fi
  case "$1" in
    -g) debug='true' ;;
    -O2) optimized='true' ;;
    -target) target_next='true' ;;
    -D__TARGET_ARCH_x86|-D__TARGET_ARCH_arm64) target_arch='true' ;;
    -I) include_next='true' ;;
    -I*)
      include="${1#-I}"
      if [ -f "${include}/vmlinux.h" ]; then generated_header='true'; fi
      if [ -f "${include}/bpf/bpf_helpers.h" ]; then libbpf_headers='true'; fi
      ;;
    *tcp_correlation_candidate.bpf.c) candidate_source='true' ;;
  esac
  if [ "$1" = '-o' ]; then
    shift
    out="$1"
  fi
  shift
done
test -n "${out}"
test "${candidate_source}" = 'true'
test "${generated_header}" = 'true'
test "${libbpf_headers}" = 'true'
test "${debug}" = 'true'
test "${optimized}" = 'true'
test "${bpf_target}" = 'true'
test "${target_arch}" = 'true'
mkdir -p "$(dirname "${out}")"
printf candidate > "${out}"
