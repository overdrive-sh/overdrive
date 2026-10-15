#!/bin/sh
# rsync wrapper for metal sync: skip build output and other spikes' scratch/evidence trees.
exec /opt/homebrew/bin/rsync \
  --exclude="/spike-scratch/netns-density-295-ka5/out/" \
  --exclude="/spike-scratch/netns-density-295-v*/" \
  --exclude=".code-review-graph/" \
  --exclude="__pycache__/" \
  --exclude="/spike-scratch/*/increment-*/evidence*/" "$@"
