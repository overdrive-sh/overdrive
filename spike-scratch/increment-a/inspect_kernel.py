#!/usr/bin/env python3
"""PROBE increment-a — identify what /boot/vmlinuz-* actually is on this host.

CH's aarch64 loader (vmm/src/vm.rs::load_kernel) tries linux_loader's PE loader
first, which validates the arm64 Linux Image magic "ARM\\x64" at offset 0x38.
On InvalidImageMagicNumber it falls back to load_uefi(), which caps at 3 MiB.
A 23 MB EFI-zboot vmlinuz therefore dies as UefiTooBig. This script establishes
which wrapper we are dealing with and where the real payload lives.
"""
import struct
import sys

K = sys.argv[1] if len(sys.argv) > 1 else "/boot/vmlinuz-7.0.0-28-generic"
d = open(K, "rb").read()
print(f"file           : {K}")
print(f"size           : {len(d)} bytes")
print(f"arm64 Image magic @0x38 : {d[0x38:0x3c]!r} (expected b'ARM\\x64')")
print(f"image_size     @0x10    : 0x{struct.unpack_from('<Q', d, 0x10)[0]:x}")

peoff = struct.unpack_from("<I", d, 0x3c)[0]
assert d[peoff:peoff + 4] == b"PE\0\0", d[peoff:peoff + 4]
mach, nsec, _ts, _ps, _ns, optsz, _ch = struct.unpack_from("<HHIIIHH", d, peoff + 4)
print(f"PE machine     : 0x{mach:x}  sections={nsec}  opt_hdr={optsz}")

sect = peoff + 24 + optsz
hdr = "%-12s %10s %10s %10s %10s" % ("name", "vsize", "vaddr", "rawsize", "rawptr")
print(hdr)
dtb_count = 0
dtb_bytes = 0
for i in range(nsec):
    o = sect + i * 40
    name = d[o:o + 8].rstrip(b"\0").decode(errors="replace")
    vs, va, rs, rp = struct.unpack_from("<IIII", d, o + 8)
    if name.startswith(".dtbauto"):
        dtb_count += 1
        dtb_bytes += rs
        continue
    print("%-12s 0x%08x 0x%08x 0x%08x 0x%08x" % (name, vs, va, rs, rp))
print(f"(+ {dtb_count} .dtbauto sections totalling {dtb_bytes} bytes)")

z = d.find(b"zimg")
print()
if z < 0:
    print("no 'zimg' marker found")
    sys.exit(0)
print(f"'zimg' marker at file offset 0x{z:x} ({z})")
print(f"  bytes[{z-4}:{z+64}] = {d[z-4:z+64]!r}")
# EFI zboot header (drivers/firmware/efi/libstub/zboot-header.S):
#   0x00 MZ magic | 0x04 "zimg" | 0x08 payload_offset u32 | 0x0c payload_size u32
#   0x18 compression type (8 bytes ascii, e.g. "gzip", "zstd22")
base = z - 4
if d[base:base + 2] == b"MZ":
    poff, psize = struct.unpack_from("<II", d, base + 8)
    comp = d[base + 0x18:base + 0x20].rstrip(b"\0").decode(errors="replace")
    print(f"  NESTED EFI zboot image at 0x{base:x}")
    print(f"    payload_offset=0x{poff:x} payload_size=0x{psize:x} comp={comp!r}")
    start = base + poff
    print(f"    payload file range = [0x{start:x}, 0x{start+psize:x})")
    print(f"    payload first 8 bytes = {d[start:start+8].hex(' ')}")
    open("/var/tmp/spike-increment-a/payload.bin", "wb").write(d[start:start + psize])
    print("    wrote /var/tmp/spike-increment-a/payload.bin")
else:
    print(f"  bytes before marker are not MZ: {d[base:base+2]!r}")
