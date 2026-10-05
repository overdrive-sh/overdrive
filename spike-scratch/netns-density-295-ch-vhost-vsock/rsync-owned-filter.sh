#!/bin/sh
# rsync wrapper for metal sync: never touch root-owned per-increment evidence
# directories that runs (this spike's and earlier spikes') leave on the host.
exec /opt/homebrew/bin/rsync --exclude=.code-review-graph/ --exclude="/spike-scratch/*/increment-*/evidence/" "$@"
