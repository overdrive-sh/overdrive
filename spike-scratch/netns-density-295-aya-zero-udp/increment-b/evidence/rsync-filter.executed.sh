#!/bin/sh
exec /opt/homebrew/bin/rsync --exclude=.code-review-graph/ "$@"
