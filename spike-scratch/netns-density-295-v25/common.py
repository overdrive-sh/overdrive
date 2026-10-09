"""Shared helpers for the V-25 probe (host-side, root, native metal)."""
import errno, fcntl, json, os, pathlib, struct, subprocess, sys, time

DEV = "/dev/vhost-vsock"
VHOST_VSOCK_SET_GUEST_CID = 0x4008AF60   # _IOW(0xAF, 0x60, u64)
VHOST_VSOCK_SET_RUNNING = 0x4004AF61     # _IOW(0xAF, 0x61, int)
VHOST_SET_OWNER = 0xAF01                 # _IO(0xAF, 0x01)
VHOST_RESET_OWNER = 0xAF02
CID_BASE = 3250                          # this spike owns CIDs 3250..3259 only


def open_dev(cloexec=True):
    flags = os.O_RDWR | os.O_NONBLOCK | (os.O_CLOEXEC if cloexec else 0)
    return os.open(DEV, flags)


def claim(fd, cid):
    """VHOST_VSOCK_SET_GUEST_CID. Returns 0 on success, else errno."""
    assert CID_BASE <= cid <= CID_BASE + 9 or cid in (0, 1, 2, 0xFFFFFFFF, 1 << 32, 3), cid
    try:
        fcntl.ioctl(fd, VHOST_VSOCK_SET_GUEST_CID, struct.pack("Q", cid))
        return 0
    except OSError as e:
        return e.errno


def set_owner(fd):
    try:
        fcntl.ioctl(fd, VHOST_SET_OWNER, 0)
        return 0
    except OSError as e:
        return e.errno


def ename(rc):
    return "OK" if rc == 0 else errno.errorcode.get(rc, str(rc))


def claim_fresh(cid):
    """Claim cid on a brand-new instance. Returns (rc, fd_or_None). Caller closes fd on rc==0."""
    fd = open_dev()
    rc = claim(fd, cid)
    if rc != 0:
        os.close(fd)
        return rc, None
    return 0, fd


def claim_poll(cid, limit_s=1.0):
    """Claim cid on fresh instances until one succeeds. Returns (latency_s, attempts, fd)."""
    t0 = time.perf_counter()
    n = 0
    while True:
        n += 1
        rc, fd = claim_fresh(cid)
        if rc == 0:
            return time.perf_counter() - t0, n, fd
        if rc != errno.EADDRINUSE:
            raise RuntimeError(f"unexpected claim errno {ename(rc)}")
        if time.perf_counter() - t0 > limit_s:
            return None, n, None
