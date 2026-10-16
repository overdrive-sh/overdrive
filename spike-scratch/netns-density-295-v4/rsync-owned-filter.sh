#!/bin/sh
# rsync wrapper for metal sync: ship only this spike's sources; skip other spikes' scratch/evidence/build trees.
exec /opt/homebrew/bin/rsync \
  --include="/spike-scratch/netns-density-295-v4/" \
  --exclude="/spike-scratch/netns-density-295-v4/out/" \
  --exclude="/spike-scratch/netns-density-295-ka5/out/" \
  --exclude="/spike-scratch/netns-density-295-v*/" \
  --exclude=".code-review-graph/" \
  --exclude="__pycache__/" \
  --exclude="/spike-scratch/*/increment-*/evidence*/" "$@"
