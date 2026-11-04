# Public ingress gateway

This checked-in operator journey exposes the `api` VM Service at
`https://<PUBLIC_HOSTNAME>/` through the gateway embedded in `overdrive serve`.
The external client uses ordinary Web PKI over TLS 1.3 and HTTP/1.1. The
workload is identity-unaware: the guest speaks plaintext HTTP and holds no
certificate or key. Overdrive owns the public TLS listener, demand-gated
cgroup-BPF backend selection, the exact-peer mTLS toward the selected backend,
and cleanup.

The bundle:

| File | Role |
|---|---|
| `service.toml` | the `api` VM Service (`[vm]` kernel and rootfs, one TCP listener on 8080); never `[exec]` |
| `guest_server.rs` | dependency-free guest HTTP/1.1 server: `/stream` (chunked, `X-Workload: api`, body streamed over about three seconds), `/exact` (`Content-Length`), `/count` (request-count marker), `/ready` |
| `prepare.sh` | marker-owned materialization of the kernel, private rootfs and static guest binary under `/srv/vm/overdrive-testing/pig-example`; `check-source`, `prepare`, `check`, `cleanup` |
| `route.toml.template` | the public Route; one `__PUBLIC_HOSTNAME__` token, `target.service = "api"` |
| `prepare-test-ca.sh` | hermetic mode only: a bounded test CA, leaf certificate and key for the hostname |
| `run-example.sh` | the journey |

## Running

Run on the qualified native x86_64 metal host (real `/dev/kvm`, uid 0), with a
default-feature `overdrive` binary built beforehand:

```text
cargo xtask metal run -- examples/public-ingress-gateway/run-example.sh \
  "$(pwd)/target/debug/overdrive" /srv/vm/overdrive-testing/pig-scratch 127.0.0.1
```

The scratch directory must not exist and must be on the same filesystem as
`/srv/vm/overdrive-testing` (the rootfs is reflinked and the VMM reads it from
the serve data directory).

Arguments are positional and argv-only; no environment variable changes them:

```text
run-example.sh <absolute-overdrive-binary> <absolute-scratch-dir> <gateway-ipv4> \
  [--public-hostname <PUBLIC_HOSTNAME> \
   --certificate-chain <absolute-chain-path> \
   --private-key <absolute-key-path>]
```

With no named options the example runs hermetically: the hostname is
`api.example.com`, a test CA and certificate are generated under the scratch
directory, and the public request trusts that CA with `--cacert`. With all
three named options (all-or-none, each once) it runs in production-trust mode:
the operator-supplied hostname, leaf-first public chain and matching PKCS#8 key
(mode 0600) are used, and the public request uses normal system trust. Partial,
repeated or unknown options exit 2 before startup.

## What it does

1. Materializes the VM Service and starts `overdrive serve` with the four
   `--gateway-*` arguments (`--gateway-certified-key-id api-origin`).
2. Deploys `service.toml` and waits for the allocation to be Running.
3. Replaces the single `__PUBLIC_HOSTNAME__` token in `route.toml.template`,
   writes `<scratch>/specs/route.toml`, and deploys that file with
   `overdrive deploy`. The Route's `certified_key = "api-origin"` is the id
   given to `--gateway-certified-key-id`; `target.service = "api"` is the
   Service id. The gateway address is never written into the Route.
4. Waits for `GET /v1/gateway/status` over the operator mTLS listener to
   report `listener_bound`.
5. Requests `/stream`, `/exact` and `/count` with
   `curl --resolve <hostname>:443:<gateway-ipv4>`, TLS 1.3 only, HTTP/1.1. The
   same hostname is the URL, SNI, Host header and certificate identity.
6. Stops the Route and Service, waits for the owned VM, network and cgroup
   resources to be released, stops `serve`, removes the materialization, and
   prints the teardown deltas. A nonzero delta fails the run.

Production requires an operator-managed A record from the Public Hostname to the
gateway address. The example never provisions DNS and never edits `/etc/hosts`;
`--resolve` is a client-local address override only.

All commands, statuses, headers, bodies, timings and the `serve` log are kept
under the scratch directory (`observations/`, `serve.log`).

This is an operator example, not a regression oracle. The recurring check is the
in-process end-to-end test; the point-in-time capture of this journey is
expectation E15 (`verification/expectations/E15-public-ingress-operator-journey/`).
