#!/usr/bin/env python3
"""Validate request receipt occurrences from one AT-PIG-E2E-1 run."""

import ipaddress
import pathlib
import re
import sys


trace_dir = pathlib.Path(sys.argv[1])
observations = pathlib.Path(sys.argv[2])
describe_path = pathlib.Path(sys.argv[3])
strace = "\n".join(path.read_text(errors="replace") for path in trace_dir.glob("serve*"))
assert "SO_COOKIE" in strace, "production connector never reached the SO_COOKIE syscall"

describe = describe_path.read_text(errors="replace")
identities = re.findall(
    r"spiffe_id:\s+(spiffe://overdrive\.local/workload/api/alloc/[a-z0-9._-]+)",
    describe,
)
assert len(set(identities)) == 1, "api must expose one exact applied workload identity"
applied_identity = identities[0]


def field(line: str, name: str) -> str | None:
    """Return one structured tracing field from a rendered event line."""
    match = re.search(rf'\b{name}=(?:"([^"]*)"|([^\s]+))', line)
    return None if match is None else (match.group(1) or match.group(2)).rstrip(",")


def completed(kind: str, status: str) -> pathlib.Path:
    """Select the sole completed request observation for a status."""
    candidates = []
    for record in sorted(observations.glob(f"*-public-request-{kind}")):
        metadata = (record / "metadata").read_text(errors="replace")
        if re.search(rf"^status=0/{status}$", metadata, re.MULTILINE):
            candidates.append(record)
    assert len(candidates) == 1, f"expected one completed {kind}/{status} observation"
    return candidates[0]


expected = {"200": "selected", "404": None, "502": "selected", "503": "no_backend"}
selected_backend_ids = set()
for kind, outcome in expected.items():
    record = completed(kind, kind)
    suffix = (record / "serve-log-suffix").read_text(errors="replace")
    request_lines = [
        line
        for line in suffix.splitlines()
        if field(line, "event") == "gateway.upstream.receipt.validated"
        and field(line, "operation") == "request"
    ]
    if outcome is None:
        assert not request_lines, "Route-miss 404 must not enter the gateway connector"
        continue
    assert len(request_lines) == 1, (
        f"{kind} suffix must contain exactly one request receipt occurrence"
    )
    line = request_lines[0]
    assert field(line, "receipt_outcome") == outcome
    cookie = int(field(line, "socket_cookie"))
    assert cookie > 0
    ipaddress.ip_address(field(line, "service_key_vip"))
    assert field(line, "service_key_port") == "8080"
    assert field(line, "service_key_protocol") == "tcp"
    assert "validated gateway upstream BPF selection receipt" in line
    if outcome == "selected":
        backend_id = int(field(line, "backend_id"))
        assert backend_id > 0
        selected_backend_ids.add(backend_id)
        assert field(line, "expected_peer_spiffe_id") == applied_identity
    else:
        assert field(line, "backend_id") is None
        assert field(line, "expected_peer_spiffe_id") is None
assert selected_backend_ids, "Selected request receipt never exposed a BackendId"
