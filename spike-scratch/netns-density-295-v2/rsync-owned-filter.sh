#!/bin/sh
# rsync wrapper for metal sync: ship this probe's sources; skip build output, evidence and other spikes.
exec /opt/homebrew/bin/rsync \
  --exclude="/spike-scratch/netns-density-295-v2/out/" \
  --exclude="/spike-scratch/*/increment-*/evidence*/" \
  --exclude="/spike-scratch/netns-density-295-ka5/out/" \
  --include="/spike-scratch/netns-density-295-v2/" \
  --exclude="/spike-scratch/netns-density-295-v*/" \
  --exclude=".code-review-graph/" \
  --exclude="__pycache__/" "$@"
