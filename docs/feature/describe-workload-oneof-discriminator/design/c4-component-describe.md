# C4 — Describe projection path (Component, L3)

GH #183 / ADR-0064. This change adds **no new container** — the L1
System Context and L2 Container topology are unchanged from brief.md
§ 67 (the `overdrive-control-plane` container already holds both the
`IntentStore` and the `ServiceVipAllocator`). The only new edge is the
read-only `describe_workload → ServiceVipAllocator::get` lookup for the
Service arm. A component-level view of the describe projection path:

```mermaid
C4Component
  title Component — describe_workload projection path (GET /v1/jobs/{id})

  Person(operator, "Operator", "Runs `overdrive describe <id>`")

  Container_Boundary(cp, "overdrive-control-plane") {
    Component(handler, "describe_workload handler", "axum handler", "Reads intent, resolves VIP, projects DescribeSpecOutput")
    Component(describe, "DescribeSpecOutput projection", "overdrive-core::api::describe + to_describe()", "oneOf{ Job | Service | Schedule } render")
    ComponentDb(intent, "IntentStore", "redb", "Persisted WorkloadIntent (rkyv envelope)")
    Component(alloc, "ServiceVipAllocator", "in-memory memo + redb-backed", "Platform-issued VIP, keyed by spec_digest")
  }

  Rel(operator, handler, "GET /v1/jobs/{id} via")
  Rel(handler, intent, "reads WorkloadIntent from")
  Rel(handler, alloc, "reads VIP (read-only get) from")
  Rel(handler, describe, "projects intent + vip into")
  Rel(describe, operator, "returns oneOf JSON response to")
```

Notes:

- Every arrow is verb-labelled; no abstraction-level mixing (all nodes
  are components within the one control-plane container).
- The `handler → alloc` edge is the single new component-level edge this
  feature introduces. It is **read-only** (`get`, not `allocate`) per
  OQ-7; the allocator state is never mutated on a describe (GET).
- Service arm only: a `WorkloadIntent::Job` describe never touches the
  allocator; a `WorkloadIntent::Schedule` is rejected before the
  allocator read (Phase 1 cannot persist a Schedule).
- The `describe → operator` response is the `oneOf`-discriminated
  `DescribeSpecOutput` (`kind: job | service | schedule`); the Service
  arm carries the required `vip` resolved from the allocator.
