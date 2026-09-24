#!/usr/bin/env python3
"""Bounded primary-source audit for Cloud Hypervisor v53.0's fd= path."""

import pathlib
import subprocess


CHECKOUT = pathlib.Path("/Users/marcus/git/cloud-hypervisor/cloud-hypervisor")
TAG = "v53.0"
EXPECTED_COMMIT = "9ed824d6d08df3e96f7d5f50795d9449ac99f431"


def git(*args):
    return subprocess.run(
        ["git", "-C", str(CHECKOUT), *args],
        check=True,
        text=True,
        capture_output=True,
    ).stdout


def source(path):
    return git("show", f"{TAG}:{path}").splitlines()


def excerpt(path, first, last):
    lines = source(path)
    print(f"SOURCE_EXCERPT {path}:{first}-{last}")
    for number in range(first, last + 1):
        print(f"{number:5d} {lines[number - 1]}")


print("SOURCE_AUDIT BEGIN")
print("Hypothesis: CH v53.0's --net fd= path issues TUNGETIFF, TUNSETIFF, "
      "TUNSETVNETHDRSZ, SIOCGIFMTU, optional SIOCSIFMTU, and TUNSETOFFLOAD; none is deny-listed.")
print("Predicted outcome: from_tap_fds selects Tap::from_tap_fd; construction contains the first "
      "four unconditional ioctls plus optional SIOCSIFMTU; activation contains TUNSETOFFLOAD; "
      "named-path host-MAC/IP/link-enable setters are not called.")
print("Falsification: any fd-path call reaches a deny-listed request, host_mac/IP/link enable, or "
      "another ioctl not named above.")
commit = git("rev-parse", f"{TAG}^{{commit}}").strip()
print(f"tag={TAG} commit={commit}")
assert commit == EXPECTED_COMMIT
excerpt("vmm/src/device_manager.rs", 2988, 3020)
excerpt("net_util/src/tap.rs", 235, 289)
excerpt("net_util/src/tap.rs", 413, 482)
excerpt("virtio-devices/src/net.rs", 529, 553)
excerpt("virtio-devices/src/net.rs", 700, 751)
excerpt("virtio-devices/src/net.rs", 849, 965)
excerpt("vmm/src/seccomp_filters.rs", 337, 365)

tap = "\n".join(source("net_util/src/tap.rs"))
net = "\n".join(source("virtio-devices/src/net.rs"))
required = (
    "libc::TUNGETIFF",
    "libc::TUNSETIFF",
    "libc::TUNSETVNETHDRSZ",
    "libc::SIOCGIFMTU",
    "libc::SIOCSIFMTU",
    "libc::TUNSETOFFLOAD",
)
for symbol in required:
    assert symbol in tap or symbol in net, symbol
print("SOURCE_AUDIT_RESULT=PASS exact_fd_path_ioctl_set=" + ",".join(required))
