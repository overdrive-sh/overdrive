#!/usr/bin/env python3
"""K-A5 host-side holder: a SEPARATE host process that owns the vsock listener (and, in 'accept'
mode, the accepted connection). The harness SIGKILLs it. usage: holder.py PORT accept|noaccept"""
import socket, sys, time, os

port, mode = int(sys.argv[1]), sys.argv[2]
s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
s.bind((socket.VMADDR_CID_ANY, port))
s.listen(128)
os.write(1, b"L\n")
if mode == "accept":
    c, _ = s.accept()
    os.write(1, b"A\n")
while True:
    time.sleep(3600)
