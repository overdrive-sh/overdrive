//! The canary verdict (DDD-5): did the host's loopback connect cost shift between the start and
//! the end of the run?
//!
//! The end set is compared with the start set under the same verdict rule as an arm against the
//! reference, with the A/A floor taken from the alternating halves of the start set. Sampling is
//! the runner's; this module only reads the samples it is given.

use rand::SeedableRng;
use rand::rngs::StdRng;

use crate::stats::{median, shift};

/// The canary's verdict over a run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CanaryVerdict {
    Steady,
    Unstable { start_median: f64, end_median: f64 },
}

/// The verdict for `end` against `start`, or `None` when there is no basis for one: the A/A
/// floor needs a start sample for each alternating half, and the end set must not be empty.
pub fn verdict(start: &[f64], end: &[f64], seed: u64) -> Option<CanaryVerdict> {
    if start.len() < 2 || end.is_empty() {
        return None;
    }
    let even: Vec<f64> = start.iter().copied().step_by(2).collect();
    let odd: Vec<f64> = start.iter().copied().skip(1).step_by(2).collect();
    let mut rng = StdRng::seed_from_u64(seed);
    let measured = shift(&mut rng, start, end, &even, &odd);
    Some(if measured.distinguishable() {
        CanaryVerdict::Unstable { start_median: median(start), end_median: median(end) }
    } else {
        CanaryVerdict::Steady
    })
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss, reason = "tests build small exactly representable samples")]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn samples(base: f64) -> Vec<f64> {
        (0..16).map(|i| base + f64::from(i)).collect()
    }

    #[test]
    fn identical_start_and_end_sets_are_steady() {
        assert_eq!(verdict(&samples(1000.0), &samples(1000.0), 1), Some(CanaryVerdict::Steady));
    }

    #[test]
    fn a_shift_far_beyond_the_start_sets_own_spread_is_unstable_and_names_both_medians() {
        assert_eq!(
            verdict(&samples(1000.0), &samples(2000.0), 1),
            Some(CanaryVerdict::Unstable { start_median: 1007.5, end_median: 2007.5 })
        );
        assert_eq!(
            verdict(&samples(2000.0), &samples(1000.0), 1),
            Some(CanaryVerdict::Unstable { start_median: 2007.5, end_median: 1007.5 }),
            "a drop is a shift too"
        );
    }

    #[test]
    fn a_shift_inside_the_spread_of_the_start_halves_is_steady() {
        let start = samples(1000.0);
        let end: Vec<f64> = start.iter().map(|value| value + 1.0).collect();
        assert_eq!(verdict(&start, &end, 1), Some(CanaryVerdict::Steady));
    }

    #[test]
    fn the_floor_comes_from_the_alternating_halves_of_the_start_set() {
        // The halves differ by 100 (evens 1000, odds 1100): a noisy host. A delta of 150 is a shift
        // from a quiet start but lies inside what the start set's own halves disagree about.
        let noisy_start: Vec<f64> =
            (0..16).map(|i| if i % 2 == 0 { 1000.0 } else { 1100.0 }).collect();
        let end: Vec<f64> = noisy_start.iter().map(|value| value + 60.0).collect();
        assert_eq!(verdict(&noisy_start, &end, 1), Some(CanaryVerdict::Steady));
        let far: Vec<f64> = noisy_start.iter().map(|value| value + 600.0).collect();
        assert!(matches!(verdict(&noisy_start, &far, 1), Some(CanaryVerdict::Unstable { .. })));
    }

    #[test]
    fn there_is_no_verdict_without_a_floor_or_an_end_set() {
        assert_eq!(verdict(&[1.0], &samples(1.0), 1), None, "one start sample has no halves");
        assert_eq!(verdict(&[], &samples(1.0), 1), None);
        assert_eq!(verdict(&samples(1.0), &[], 1), None);
        assert!(verdict(&[1.0, 2.0], &[1.0], 1).is_some(), "two start samples are enough");
    }

    proptest! {
        #[test]
        fn the_verdict_is_deterministic_under_the_seed(
            start in proptest::collection::vec(1.0f64..1e6, 2..20),
            end in proptest::collection::vec(1.0f64..1e6, 1..20),
            seed in any::<u64>(),
        ) {
            prop_assert_eq!(verdict(&start, &end, seed), verdict(&start, &end, seed));
        }

        #[test]
        fn a_run_compared_with_itself_is_steady(
            start in proptest::collection::vec(1.0f64..1e6, 2..20),
            seed in any::<u64>(),
        ) {
            prop_assert_eq!(verdict(&start, &start, seed), Some(CanaryVerdict::Steady));
        }
    }
}
