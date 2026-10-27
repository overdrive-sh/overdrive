#!/usr/bin/env bash
set -euo pipefail

probe_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
uname -r
cc -O2 -Wall -Wextra -Werror "$probe_dir/postfin_reset.c" -o "$probe_dir/postfin_reset"
sudo -n "$probe_dir/postfin_reset"
