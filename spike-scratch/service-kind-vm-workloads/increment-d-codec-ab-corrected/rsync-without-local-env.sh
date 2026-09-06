#!/usr/bin/env bash
set -euo pipefail

exec /opt/homebrew/bin/rsync \
  --exclude='/.env' \
  --exclude='/.context/service-vm-spike/target/' \
  --exclude='/spike-scratch/service-kind-vm-workloads/**/target/' \
  --exclude='/spike-scratch/service-kind-vm-workloads/**/out/' \
  "$@"
