//! F1 K3 determinism guard for `SimCgroupFs` per ADR-0054 § Sim
//! adapter (step 01-03) and `.claude/rules/testing.md` § Tier 1 /
//! "Seeding and reproducibility".
//!
//! K3 (seed → bit-identical trajectory): two fresh `SimCgroupFs`
//! instances given the same op sequence produce bit-identical Result
//! trajectories AND bit-identical final snapshots. The structural
//! defense behind ADR-0054 § D4 (method-entry deterministic) — the
//! only nondeterminism source is the `BTreeMap`-keyed schedule, whose
//! iteration order is `Ord`-deterministic.

use std::path::PathBuf;
use std::sync::Arc;

use overdrive_core::traits::CgroupFs;
use overdrive_sim::SimCgroupFs;
use proptest::collection::vec;
use proptest::prelude::*;

/// Bounded op-space: a finite path pool (8 paths) keeps the generator
/// dense enough that operations collide on the same keys frequently
/// — exactly the case where iteration-order nondeterminism would
/// surface a regression.
#[derive(Clone, Debug)]
enum Op {
    CreateDir(usize),
    Write(usize, Vec<u8>),
    RemoveDir(usize),
}

const PATH_POOL: &[&str] = &["/a", "/a/b", "/a/b/c", "/a/b/d", "/a/x", "/a/x/y", "/p", "/p/q"];

fn path_at(idx: usize) -> PathBuf {
    PathBuf::from(PATH_POOL[idx % PATH_POOL.len()])
}

fn op_strategy() -> impl Strategy<Value = Op> {
    let idx = 0usize..PATH_POOL.len();
    prop_oneof![
        idx.clone().prop_map(Op::CreateDir),
        (idx.clone(), vec(any::<u8>(), 0..16)).prop_map(|(i, b)| Op::Write(i, b)),
        idx.prop_map(Op::RemoveDir),
    ]
}

fn apply_sync(
    rt: &tokio::runtime::Runtime,
    fs: &Arc<dyn CgroupFs>,
    op: &Op,
) -> Result<(), std::io::ErrorKind> {
    let path = match op {
        Op::CreateDir(i) | Op::Write(i, _) | Op::RemoveDir(i) => path_at(*i),
    };
    let result = match op {
        Op::CreateDir(_) => rt.block_on(fs.create_dir(&path)),
        Op::Write(_, bytes) => rt.block_on(fs.write(&path, bytes)),
        Op::RemoveDir(_) => rt.block_on(fs.remove_dir(&path)),
    };
    result.map_err(|e| e.kind())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    #[test]
    fn k3_two_runs_same_seed_produce_identical_trajectory(
        ops in vec(op_strategy(), 0..100),
    ) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("tokio runtime");

        let sim_a = Arc::new(SimCgroupFs::new());
        let sim_b = Arc::new(SimCgroupFs::new());
        let fs_a: Arc<dyn CgroupFs> = sim_a.clone();
        let fs_b: Arc<dyn CgroupFs> = sim_b.clone();

        let trace_a: Vec<_> = ops.iter().map(|op| apply_sync(&rt, &fs_a, op)).collect();
        let trace_b: Vec<_> = ops.iter().map(|op| apply_sync(&rt, &fs_b, op)).collect();

        prop_assert_eq!(trace_a, trace_b, "Result trajectory must be bit-identical");
        prop_assert_eq!(
            sim_a.snapshot(),
            sim_b.snapshot(),
            "Final snapshot must be bit-identical"
        );
    }
}
