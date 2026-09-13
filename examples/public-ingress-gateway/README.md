# Public ingress gateway

This checked-in operator journey exposes the api Service at
https://api.example.com/ through the gateway embedded in overdrive serve. The
external client uses ordinary Web PKI. The identity-unaware workload sees
plain HTTP while Overdrive owns demand-gated cgroup-BPF selection, the
BackendId receipt, gateway-SVID exact-peer mTLS, and cleanup.

Production requires an operator-managed IPv4 A record from the Route hostname
to the configured gateway address. Overdrive does not provision public DNS or
edit /etc/hosts. This hermetic example instead uses curl --resolve so the
socket targets 127.0.0.1 while SNI, Host, and certificate validation remain
api.example.com.

Run it on Linux as root with a default-feature binary built before invocation:

    examples/public-ingress-gateway/run-example.sh \
      "$(pwd)/target/debug/overdrive" \
      /tmp/overdrive-public-ingress-example \
      127.0.0.1

That form creates a bounded test CA under the supplied scratch directory. For
production trust, append the all-or-none options `--public-hostname <host>`,
`--certificate-chain <absolute-path>`, and `--private-key <absolute-path>`.
The key fixture must have mode 0600. Public HTTPS then uses normal system trust.

The script invokes only production surfaces: overdrive serve with the four
gateway arguments, Service and Route deploy, operator-mTLS gateway status, and
external TLS 1.3 / HTTP/1.1 to the selected Public Hostname on port 443. The
checked-in Route template is materialized once under the scratch directory;
`target.service` remains `api`.

It captures commands, gateway status, public headers/body, and owned cleanup
state in the scratch directory. It is an operator example, not a regression
oracle. The recurring oracle is tests/tier3/public-ingress-gateway/runner.sh.
