#!/bin/bash
# V2: microVM clean half-close volume (strict)
cd /tmp/v31-work
ulimit -n 1048576
bash runplan2.sh vm vmclean vmclean.json
