//! The run schedule (DDD-5): blocks, each holding every arm once plus one A/A control.
//!
//! [`plan`] is a pure function of the declaration shape and the seed; the runner's execution is
//! the only impure step. Inside a block the order is a seeded shuffle, so no arm owns a position,
//! and the benchmark body never learns its block, position or role.

use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;

use crate::record::Role;

/// One run of the schedule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planned {
    pub(crate) block: u32,
    pub(crate) position: u32,
    pub(crate) role: Role,
    /// The arm to run; a control runs the reference arm (the first declared).
    pub(crate) arm: String,
}

/// The ordered runs of one profile: `replicates` blocks of every arm once, plus one control (a
/// second run of the reference arm) when there are two or more arms. `profile_index` keeps two
/// profiles of one run from sharing a shuffle.
pub fn plan(arms: &[&str], replicates: u32, profile_index: u32, seed: u64) -> Vec<Planned> {
    let Some(reference) = arms.first() else {
        return Vec::new();
    };
    let mut rng = StdRng::seed_from_u64(seed.wrapping_add(u64::from(profile_index)));
    let mut runs = Vec::with_capacity(arms.len().saturating_add(1) * replicates as usize);
    for block in 0..replicates {
        let mut members: Vec<(Role, &str)> = arms.iter().map(|arm| (Role::Arm, *arm)).collect();
        if arms.len() >= 2 {
            members.push((Role::Control, reference));
        }
        members.shuffle(&mut rng);
        for (position, (role, arm)) in (0..).zip(members) {
            runs.push(Planned { block, position, role, arm: arm.to_owned() });
        }
    }
    runs
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "tests assert their own preconditions and compare exact stored floats"
)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn block(runs: &[Planned], block: u32) -> Vec<&Planned> {
        runs.iter().filter(|run| run.block == block).collect()
    }

    #[test]
    fn two_arms_get_a_control_in_every_block() {
        let runs = plan(&["a", "b"], 9, 0, 7);
        assert_eq!(runs.len(), 27);
        for index in 0..9 {
            let members = block(&runs, index);
            assert_eq!(members.iter().filter(|run| run.role == Role::Control).count(), 1);
            assert_eq!(members.iter().filter(|run| run.arm == "b").count(), 1);
            assert_eq!(
                members.iter().filter(|run| run.arm == "a").count(),
                2,
                "arm a plus the control"
            );
        }
    }

    #[test]
    fn one_arm_has_no_control() {
        let runs = plan(&["only"], 9, 0, 7);
        assert_eq!(runs.len(), 9);
        assert!(runs.iter().all(|run| run.role == Role::Arm && run.arm == "only"));
    }

    #[test]
    fn no_arms_plans_nothing() {
        assert!(plan(&[], 9, 0, 7).is_empty());
    }

    #[test]
    fn the_same_seed_plans_the_same_order_and_the_order_varies_across_seeds() {
        let arms = ["a", "b", "c"];
        assert_eq!(plan(&arms, 20, 0, 1), plan(&arms, 20, 0, 1));
        assert_ne!(plan(&arms, 20, 0, 1), plan(&arms, 20, 0, 2));
        assert_ne!(plan(&arms, 20, 0, 1), plan(&arms, 20, 1, 1), "profiles do not share a shuffle");
    }

    proptest! {
        #[test]
        fn every_block_holds_every_arm_once_plus_the_control(
            arm_count in 1usize..6,
            replicates in 1u32..30,
            profile in 0u32..4,
            seed in any::<u64>(),
        ) {
            let names: Vec<String> = (0..arm_count).map(|n| format!("arm{n}")).collect();
            let arms: Vec<&str> = names.iter().map(String::as_str).collect();
            let runs = plan(&arms, replicates, profile, seed);
            let per_block = arm_count + usize::from(arm_count >= 2);
            prop_assert_eq!(runs.len(), per_block * replicates as usize);
            for index in 0..replicates {
                let members = block(&runs, index);
                prop_assert_eq!(members.len(), per_block);
                let mut positions: Vec<u32> = members.iter().map(|run| run.position).collect();
                positions.sort_unstable();
                prop_assert_eq!(positions, (0..u32::try_from(per_block).unwrap_or(0)).collect::<Vec<_>>());
                for arm in &arms {
                    let arm_runs = members.iter().filter(|run| run.role == Role::Arm && run.arm == *arm).count();
                    prop_assert_eq!(arm_runs, 1);
                }
                let controls: Vec<_> = members.iter().filter(|run| run.role == Role::Control).collect();
                prop_assert_eq!(controls.len(), usize::from(arm_count >= 2));
                prop_assert!(controls.iter().all(|run| run.arm == arms[0]));
            }
        }
    }
}
