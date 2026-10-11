#!/bin/bash
cd /tmp/v31-work
ulimit -n 1048576
bash runplan2.sh vm vmclean3 vmclean3.json
bash runplan2.sh vm vmkill3 vmkill.json
