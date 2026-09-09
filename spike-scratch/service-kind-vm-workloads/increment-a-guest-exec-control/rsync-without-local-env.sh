#!/usr/bin/env bash
set -euo pipefail

# Metal-spike sync guard: the workspace-root .env contains the SSH target.
# Keep it local while preserving the repository-standard rsync path.
exec /opt/homebrew/bin/rsync \
  --exclude='/.env' \
  --exclude='/.context/service-vm-spike/target/' \
  --exclude='/spike-scratch/service-kind-vm-workloads/**/target/' \
  --exclude='/spike-scratch/service-kind-vm-workloads/**/out/' \
  "$@"
