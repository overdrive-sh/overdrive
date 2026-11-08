# Test edits held out until `IntentKey::public_route_set()` exists (R-67 (6))

NOT IN THE BUILD. These edits name `IntentKey::public_route_set()`, which the 05-02 crafter adds to
`overdrive-core` (`aggregate/mod.rs`, beside the other intent-key constructors; `as_bytes()` for store
calls and watches, `as_str()` for the codec's `key` argument). In the same commit that adds it and
deletes `overdrive-gateway`'s `ROUTE_SET_KEY`, make these test edits (mechanical, bodies otherwise
untouched):

1. `crates/overdrive-control-plane/tests/integration/public_ingress_gateway_boot_shutdown.rs`
   - delete `const ROUTE_SET_KEY: &[u8] = b"public-ingress/route-set";` and its doc comment;
   - `stored_route_set` and `store_route_set_bytes`: use `IntentKey::public_route_set().as_bytes()`;
   - `envelope_holding_an_invalid_route`: pass `Some(IntentKey::public_route_set().as_str())` as the
     `key` argument instead of the spelled literal `"public-ingress/route-set"`;
   - `use overdrive_core::aggregate::IntentKey;` (check the existing `overdrive_core::aggregate` import).
   - Once `from_store_bytes` returns `Result<Self, RouteCodecError>`, the fixture's `Debug`-string check
     may be tightened to
     `assert!(matches!(refusal, RouteCodecError::InvalidRoute(RouteValidationError::Hostname(_))))`.
2. `crates/overdrive-gateway/tests/integration/public_ingress_gateway.rs`
   - delete `const ROUTE_SET_KEY: &str = "public-ingress/route-set";` (~:139);
   - replace every `ROUTE_SET_KEY.as_bytes()` (~:1266, 1308, 1341, 1356, 1783, 1799) with
     `IntentKey::public_route_set().as_bytes()` and `Some(ROUTE_SET_KEY)` (~:1267) with
     `Some(IntentKey::public_route_set().as_str())`.
3. Custody prefix (rust.md key-prefix SSOT): `CUSTODY_PREFIX` is spelled in
   `crates/overdrive-gateway/src/certified_key.rs:300` and re-spelled as a test copy at
   `public_ingress_gateway.rs:141` (used at ~:1256). The 05-02 crafter moves the prefix into core as
   the one definition (an `IntentKey` prefix constructor, per the orchestrator's assignment) and the
   test then consumes it (`scan_prefix` at ~:1256) instead of the local copy, which is deleted.
