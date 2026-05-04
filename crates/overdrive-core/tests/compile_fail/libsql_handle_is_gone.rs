//! ADR-0035 §5 — `LibsqlHandle` was deleted in `reconciler-memory-redb`
//! step 01-06. The type is gone for good; this fixture defends against
//! a future refactor re-introducing it under the same name.
//!
//! The import below names a path that no longer resolves. Compilation
//! fails with E0432 (unresolved import) — exactly the diagnostic shape
//! the runtime's collapsed contract demands.

use overdrive_core::reconciler::LibsqlHandle;

fn main() {
    let _: Option<LibsqlHandle> = None;
}
