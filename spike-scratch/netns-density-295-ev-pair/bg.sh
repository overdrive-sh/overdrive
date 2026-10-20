#!/bin/bash
# bg.sh <name> <runlist> <vmrun args...>: run a leased job in the background, console to out/<name>.console, exit code to out/<name>.rc
cd "$(dirname "$0")"
name=$1; rl=$2; shift 2
( ./retry.sh $name "cd /var/tmp/ev-pair-work && bash src/driver/build.sh >/dev/null && python3 src/driver/vmrun.py src/runlists/$rl name=$name $*" > out/$name.console 2>&1; echo $? > out/$name.rc ) </dev/null >/dev/null 2>&1 &
disown
echo started $name
