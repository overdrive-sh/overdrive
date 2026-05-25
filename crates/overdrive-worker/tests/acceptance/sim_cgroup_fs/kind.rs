//! `SimCgroupFs::kind()` returns the stable literal per ADR-0054
//! § Sim adapter (step 01-03). Operators grep on this string in
//! startup logs; the equivalence proptest at step 01-07 also asserts
//! the value is non-empty and distinct from `RealCgroupFs::kind()`.

use std::sync::Arc;

use overdrive_core::traits::CgroupFs;
use overdrive_sim::SimCgroupFs;

#[tokio::test]
async fn sim_kind_returns_stable_literal() {
    let fs: Arc<dyn CgroupFs> = Arc::new(SimCgroupFs::new());
    assert_eq!(fs.kind(), "overdrive_sim::SimCgroupFs");
}
