#!/bin/sh
# rsync wrapper for metal sync: never touch root-owned per-increment evidence
# directories or root-written __pycache__ that runs leave on the host.
exec /opt/homebrew/bin/rsync --exclude=.code-review-graph/ --exclude="/spike-scratch/*/increment-*/evidence/" --exclude="__pycache__/" "$@"
