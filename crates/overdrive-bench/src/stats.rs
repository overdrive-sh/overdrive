//! The statistical protocol, methodology version 1 (ADR-0300, DDD-5).
//!
//! Percentiles are R-7 (linear interpolation between closest ranks) over raw values or over a
//! log-linear histogram decoded exactly as DDD-5 pins it. A comparison is the difference of
//! per-replicate medians with a 95% percentile bootstrap, the A/A control's interval as the noise
//! floor, a seeded permutation Mann-Whitney test and the verdict rule. Everything is a pure
//! function of its inputs and the seeded generator it is handed.

use std::collections::BTreeMap;

use rand::Rng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;

use crate::histogram::{Histogram, bucket_of};

/// An arm, or the A/A control, with fewer `ok` replicates holding a value is not compared.
pub const MIN_REPLICATES: usize = 9;

const RESAMPLES: usize = 10_000;
const PERMUTATIONS: usize = 10_000;
const INTERVAL_LOWER: f64 = 0.025;
const INTERVAL_UPPER: f64 = 0.975;
const SUB_BUCKETS: u32 = 32;

pub const MEDIAN: f64 = 0.5;
pub const FIRST_QUARTILE: f64 = 0.25;
pub const THIRD_QUARTILE: f64 = 0.75;

// ---------------------------------------------------------------------------------------
// Percentiles.

/// R-7 over `count` ranked values, `value_at(k)` being the value at 0-based rank `k`.
///
/// `h = (n - 1) * p`, `f = floor(h)`; the value at rank `n - 1` when `f = n - 1`, else
/// `v(f) + (h - f) * (v(f + 1) - v(f))`, in that expression order.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    reason = "ranks are far below 2^53, `f` is a non-negative whole number below `n`, and DDD-5 pins the arithmetic without fused multiply-add"
)]
fn r7(count: u64, p: f64, value_at: impl Fn(u64) -> f64) -> f64 {
    let Some(last) = count.checked_sub(1) else {
        unreachable!("a percentile is taken over at least one value")
    };
    let h = last as f64 * p;
    let f = h.floor();
    let rank = f as u64;
    if rank == last {
        return value_at(last);
    }
    value_at(rank) + (h - f) * (value_at(rank + 1) - value_at(rank))
}

/// The R-7 percentile `p` (0 to 1) of `sorted`, which is ascending and not empty.
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    #[allow(clippy::cast_possible_truncation, reason = "a rank is below the slice length")]
    r7(sorted.len() as u64, p, |rank| {
        sorted.get(rank as usize).copied().unwrap_or_else(|| {
            unreachable!("rank {rank} is below the value count {}", sorted.len())
        })
    })
}

/// The median of `values`, which is not empty.
pub fn median(values: &[f64]) -> f64 {
    percentile(&sorted(values), MEDIAN)
}

/// An ascending copy of `values`.
pub fn sorted(values: &[f64]) -> Vec<f64> {
    let mut copy = values.to_vec();
    copy.sort_by(f64::total_cmp);
    copy
}

// ---------------------------------------------------------------------------------------
// Pooled distributions: raw values, or a histogram once any replicate is one.

/// Values and histograms gathered for one pooled distribution.
#[derive(Default)]
pub struct Pool {
    raw: Vec<f64>,
    histograms: Vec<Histogram>,
}

impl Pool {
    pub fn add_values(&mut self, values: &[f64]) {
        self.raw.extend_from_slice(values);
    }

    pub fn add_histogram(&mut self, histogram: &Histogram) {
        self.histograms.push(histogram.clone());
    }

    /// The pooled distribution, or `None` when nothing was added.
    pub fn finish(self) -> Option<Distribution> {
        if self.raw.is_empty() && self.histograms.is_empty() {
            return None;
        }
        if self.histograms.is_empty() {
            return Some(Distribution::Raw(sorted(&self.raw)));
        }
        Some(Distribution::Binned(Binned::pool(&self.raw, &self.histograms)))
    }
}

/// A pooled distribution: exact when every replicate stored raw values, binned otherwise.
pub enum Distribution {
    Raw(Vec<f64>),
    Binned(Binned),
}

impl Distribution {
    pub fn percentile(&self, p: f64) -> f64 {
        match self {
            Self::Raw(sorted) => percentile(sorted, p),
            Self::Binned(binned) => binned.percentile(p),
        }
    }

    pub fn count(&self) -> u64 {
        match self {
            Self::Raw(sorted) => sorted.len() as u64,
            Self::Binned(binned) => binned.count,
        }
    }

    pub const fn is_histogram(&self) -> bool {
        matches!(self, Self::Binned(_))
    }
}

/// Binned values: the sparse buckets (ascending), the exact count, minimum and maximum.
pub struct Binned {
    count: u64,
    min: f64,
    max: f64,
    buckets: Vec<(u32, u64)>,
}

impl Binned {
    /// Pool raw values and histograms (DDD-5 step 5): each raw value goes to its bucket, counts
    /// add per bucket, `min` is the least minimum, `max` the greatest maximum.
    fn pool(raw: &[f64], histograms: &[Histogram]) -> Self {
        let mut occupied = BTreeMap::<u32, u64>::new();
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        let mut count = 0u64;
        for &value in raw {
            *occupied.entry(bucket_of(value)).or_default() += 1;
            min = min.min(value);
            max = max.max(value);
            count += 1;
        }
        for histogram in histograms {
            for &(bucket, bucket_count) in &histogram.buckets {
                *occupied.entry(bucket).or_default() += bucket_count;
            }
            min = min.min(histogram.min);
            max = max.max(histogram.max);
            count += histogram.count;
        }
        Self { count, min, max, buckets: occupied.into_iter().collect() }
    }

    fn percentile(&self, p: f64) -> f64 {
        r7(self.count, p, |rank| self.value_at(rank))
    }

    /// The value at integer rank `k` (DDD-5 steps 2 and 3): the bucket's `lower` plus the
    /// rank's midpoint share of the bucket width, clamped to the stored `[min, max]`.
    #[allow(
        clippy::cast_precision_loss,
        clippy::suboptimal_flops,
        reason = "counts are far below 2^53 and DDD-5 pins the arithmetic without fused multiply-add"
    )]
    fn value_at(&self, k: u64) -> f64 {
        let mut below = 0u64;
        for &(bucket, count) in &self.buckets {
            if k < below + count {
                let (lower, upper) = bucket_range(bucket);
                let value = lower + ((k - below) as f64 + 0.5) / (count as f64) * (upper - lower);
                return value.max(self.min).min(self.max);
            }
            below += count;
        }
        unreachable!("rank {k} is below the value count {}", self.count)
    }
}

/// The `[lower, upper)` range of `bucket` (DDD-5 step 1): `[0, 1)` for bucket 0, else
/// `lower = 2^e (1 + s/32)` and `upper = 2^e (1 + (s+1)/32)`.
fn bucket_range(bucket: u32) -> (f64, f64) {
    if bucket == 0 {
        return (0.0, 1.0);
    }
    let index = bucket - 1;
    let (exponent, sub) = (index / SUB_BUCKETS, index % SUB_BUCKETS);
    let base = 2f64.powi(i32::try_from(exponent).unwrap_or(i32::MAX));
    let width = f64::from(SUB_BUCKETS);
    (base * (1.0 + f64::from(sub) / width), base * (1.0 + f64::from(sub + 1) / width))
}

// ---------------------------------------------------------------------------------------
// Comparison.

/// A closed interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interval {
    pub lower: f64,
    pub upper: f64,
}

impl Interval {
    fn excludes_zero(self) -> bool {
        self.lower > 0.0 || self.upper < 0.0
    }

    /// The larger absolute bound.
    fn reach(self) -> f64 {
        self.lower.abs().max(self.upper.abs())
    }
}

/// A shift of one set of per-replicate values against another, and the noise floor it is read against.
#[derive(Clone, Copy, Debug)]
pub struct Shift {
    pub delta: f64,
    pub interval: Interval,
    pub floor: Interval,
}

impl Shift {
    /// The verdict rule: the delta interval excludes 0 and the point estimate exceeds the larger
    /// absolute bound of the A/A interval.
    pub fn distinguishable(&self) -> bool {
        self.interval.excludes_zero() && self.delta.abs() > self.floor.reach()
    }
}

/// The delta of `other` against `reference` (the difference of medians), its 95% bootstrap
/// interval and the interval of `floor_other` against `floor_reference` (the A/A pair).
/// Every slice is not empty.
pub fn shift(
    rng: &mut StdRng,
    reference: &[f64],
    other: &[f64],
    floor_reference: &[f64],
    floor_other: &[f64],
) -> Shift {
    Shift {
        delta: median(other) - median(reference),
        interval: bootstrap_interval(rng, reference, other),
        floor: bootstrap_interval(rng, floor_reference, floor_other),
    }
}

/// The 95% percentile bootstrap interval of `median(other) - median(reference)` over
/// 10,000 resamples of the replicates of each.
fn bootstrap_interval(rng: &mut StdRng, reference: &[f64], other: &[f64]) -> Interval {
    let mut differences = Vec::with_capacity(RESAMPLES);
    let mut reference_sample = vec![0.0; reference.len()];
    let mut other_sample = vec![0.0; other.len()];
    for _ in 0..RESAMPLES {
        let from_other = resampled_median(rng, other, &mut other_sample);
        let from_reference = resampled_median(rng, reference, &mut reference_sample);
        differences.push(from_other - from_reference);
    }
    differences.sort_by(f64::total_cmp);
    Interval {
        lower: percentile(&differences, INTERVAL_LOWER),
        upper: percentile(&differences, INTERVAL_UPPER),
    }
}

fn resampled_median(rng: &mut StdRng, values: &[f64], sample: &mut [f64]) -> f64 {
    for slot in &mut *sample {
        *slot = values.get(rng.gen_range(0..values.len())).copied().unwrap_or_else(|| {
            unreachable!("the drawn index is below the value count {}", values.len())
        });
    }
    sample.sort_by(f64::total_cmp);
    percentile(sample, MEDIAN)
}

/// The Mann-Whitney U of the compared arm and its seeded two-sided permutation p-value.
#[derive(Clone, Copy, Debug)]
pub struct MannWhitney {
    pub u: f64,
    pub p: f64,
}

/// U of `arm` against `reference` (midranks for ties), and the p-value from 10,000 seeded
/// permutations of the labels: `(extreme + 1) / (permutations + 1)`, where a permutation is
/// extreme when its U lies at least as far from the null mean as the observed one.
#[allow(clippy::cast_precision_loss, reason = "sample sizes are far below 2^53")]
fn mann_whitney(rng: &mut StdRng, arm: &[f64], reference: &[f64]) -> MannWhitney {
    let combined: Vec<f64> = arm.iter().chain(reference).copied().collect();
    let mut ranks = midranks(&combined);
    let arm_size = arm.len();
    let base = (arm_size * (arm_size + 1)) as f64 / 2.0;
    let mean = (arm_size * reference.len()) as f64 / 2.0;
    let u_of = |ranks: &[f64]| ranks.iter().take(arm_size).sum::<f64>() - base;
    let u = u_of(&ranks);
    let observed = (u - mean).abs();
    let mut extreme = 0usize;
    for _ in 0..PERMUTATIONS {
        ranks.shuffle(rng);
        if (u_of(&ranks) - mean).abs() >= observed {
            extreme += 1;
        }
    }
    MannWhitney { u, p: (extreme + 1) as f64 / (PERMUTATIONS + 1) as f64 }
}

/// The 1-based rank of each value among all of them, tied values sharing their mean rank.
#[allow(clippy::cast_precision_loss, reason = "ranks are far below 2^53")]
fn midranks(values: &[f64]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|a, b| values[*a].total_cmp(&values[*b]));
    let mut ranks = vec![0.0; values.len()];
    let mut start = 0;
    while start < order.len() {
        let mut end = start + 1;
        while end < order.len() && values[order[end]].total_cmp(&values[order[start]]).is_eq() {
            end += 1;
        }
        let shared = (start + 1 + end) as f64 / 2.0;
        for &index in &order[start..end] {
            ranks[index] = shared;
        }
        start = end;
    }
    ranks
}

/// What the protocol says about one arm against the reference.
#[derive(Debug)]
pub enum Comparison {
    /// An arm, or the A/A control, has fewer than [`MIN_REPLICATES`] values: no comparison is made.
    Insufficient,
    Compared {
        shift: Shift,
        test: MannWhitney,
    },
}

/// Compare `arm` with `reference` against the A/A `control` (a second run of the reference arm),
/// each the per-replicate values of the `ok` replicates holding one.
pub fn compare(rng: &mut StdRng, reference: &[f64], arm: &[f64], control: &[f64]) -> Comparison {
    if [reference.len(), arm.len(), control.len()].iter().any(|held| *held < MIN_REPLICATES) {
        return Comparison::Insufficient;
    }
    let measured = shift(rng, reference, arm, reference, control);
    let test = mann_whitney(rng, arm, reference);
    Comparison::Compared { shift: measured, test }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::suboptimal_flops,
    reason = "tests compare exactly representable values, assert their own preconditions and restate DDD-5's unfused arithmetic"
)]
mod tests {
    use proptest::prelude::*;
    use rand::SeedableRng;

    use super::*;

    fn rng(seed: u64) -> StdRng {
        StdRng::seed_from_u64(seed)
    }

    fn ramp(from: f64, count: usize) -> Vec<f64> {
        (0..count).map(|i| from + i as f64).collect()
    }

    #[test]
    fn percentiles_interpolate_linearly_between_closest_ranks() {
        let one_to_ten = ramp(1.0, 10);
        for (p, expected) in
            [(0.0, 1.0), (0.5, 5.5), (0.9, 9.1), (0.95, 9.55), (0.99, 9.91), (1.0, 10.0)]
        {
            assert!((percentile(&one_to_ten, p) - expected).abs() < 1e-9, "p{p}");
        }
        assert_eq!(percentile(&[7.0], 0.99), 7.0, "a single value is every percentile");
        assert_eq!(percentile(&[1.0, 3.0], 0.5), 2.0);
        assert_eq!(median(&[9.0, 1.0, 5.0]), 5.0, "the median sorts first");
        assert_eq!(percentile(&ramp(1000.0, 9), 0.25), 1002.0, "quartile at an integral rank");
        assert_eq!(percentile(&ramp(1000.0, 9), 0.75), 1006.0);
    }

    proptest! {
        #[test]
        fn a_percentile_stays_within_the_values_and_grows_with_p(
            values in proptest::collection::vec(0.0f64..1e9, 1..60),
            a in 0.0f64..=1.0,
            b in 0.0f64..=1.0,
        ) {
            let ordered = sorted(&values);
            let (low, high) = if a <= b { (a, b) } else { (b, a) };
            let (low_value, high_value) = (percentile(&ordered, low), percentile(&ordered, high));
            prop_assert!(low_value <= high_value);
            prop_assert!(low_value >= ordered[0] && high_value <= ordered[ordered.len() - 1]);
            prop_assert_eq!(percentile(&ordered, 0.0), ordered[0]);
            prop_assert_eq!(percentile(&ordered, 1.0), ordered[ordered.len() - 1]);
        }
    }

    fn pooled(raw: &[f64], histograms: &[&[f64]]) -> Distribution {
        let mut pool = Pool::default();
        pool.add_values(raw);
        for values in histograms {
            pool.add_histogram(&Histogram::from_values(values).expect("storable"));
        }
        pool.finish().expect("a distribution")
    }

    #[test]
    fn an_empty_pool_has_no_distribution() {
        assert!(Pool::default().finish().is_none());
    }

    #[test]
    fn raw_values_pool_exactly_and_a_histogram_makes_the_pool_binned() {
        let raw = pooled(&[3.0, 1.0, 2.0], &[]);
        assert!(!raw.is_histogram());
        assert_eq!(raw.count(), 3);
        assert_eq!(raw.percentile(0.5), 2.0);
        let thousand: Vec<f64> = ramp(1.0, 1500);
        let binned = pooled(&[2000.0], &[&thousand]);
        assert!(binned.is_histogram());
        assert_eq!(binned.count(), 1501);
    }

    #[test]
    fn a_binned_percentile_follows_the_pinned_computation() {
        // Bucket 1 is [1, 1.03125): four values at ranks 0..=3 sit at 1 + (k + 0.5) / 4 * 0.03125.
        let binned = Binned { count: 4, min: 1.0, max: 1.5, buckets: vec![(1, 4)] };
        let rank = |k: f64| 1.0 + (k + 0.5) / 4.0 * 0.031_25;
        assert_eq!(binned.value_at(0), rank(0.0));
        assert_eq!(binned.value_at(3), rank(3.0));
        assert_eq!(binned.percentile(1.0), rank(3.0));
        assert_eq!(binned.percentile(0.0), rank(0.0));
        let h = 3.0 * 0.5;
        assert_eq!(binned.percentile(0.5), rank(1.0) + (h - 1.0) * (rank(2.0) - rank(1.0)));
    }

    #[test]
    fn a_binned_value_is_clamped_to_the_stored_extremes() {
        let low = Binned { count: 2, min: 1.02, max: 1.022, buckets: vec![(1, 2)] };
        assert_eq!(low.value_at(0), 1.02, "below min is raised to it");
        assert_eq!(low.value_at(1), 1.022, "above max is lowered to it");
    }

    #[test]
    fn a_binned_rank_crosses_buckets_by_the_count_below() {
        let binned = Binned { count: 3, min: 0.0, max: 3.0, buckets: vec![(0, 2), (33, 1)] };
        assert_eq!(binned.value_at(0), 0.25);
        assert_eq!(binned.value_at(1), 0.75);
        assert_eq!(
            binned.value_at(2),
            2.0 + 0.5 * (2.0 * (1.0 + 1.0 / 32.0) - 2.0),
            "bucket 33 is [2, 2.0625)"
        );
    }

    #[test]
    fn bucket_ranges_follow_the_binary_exponent_and_the_top_five_fraction_bits() {
        assert_eq!(bucket_range(0), (0.0, 1.0));
        assert_eq!(bucket_range(1), (1.0, 1.031_25));
        assert_eq!(bucket_range(32), (1.968_75, 2.0));
        assert_eq!(bucket_range(33), (2.0, 2.0625));
        assert_eq!(bucket_range(351), (1984.0, 2016.0));
        assert_eq!(bucket_range(2048).0, 2f64.powi(63) * (1.0 + 31.0 / 32.0));
    }

    proptest! {
        #[test]
        fn a_binned_percentile_stays_within_the_stated_error_of_the_exact_one(
            values in proptest::collection::vec(1.0f64..1e7, 1001..1400),
            p in 0.0f64..=1.0,
        ) {
            let exact = percentile(&sorted(&values), p);
            let binned = pooled(&[], &[&values]).percentile(p);
            prop_assert!((binned - exact).abs() <= 0.03125 * exact, "{binned} vs {exact}");
        }
    }

    #[test]
    fn a_pool_of_raw_values_and_histograms_counts_and_bounds_them_all() {
        let big: Vec<f64> = ramp(100.0, 1100);
        let distribution = pooled(&[5.0, 90_000.0], &[&big]);
        assert_eq!(distribution.count(), 1102);
        assert!(distribution.percentile(1.0) <= 90_000.0);
        assert!(distribution.percentile(0.0) >= 5.0);
        let Distribution::Binned(binned) = distribution else { panic!("binned") };
        assert_eq!((binned.min, binned.max), (5.0, 90_000.0));
    }

    #[test]
    fn midranks_share_the_mean_rank_among_ties() {
        assert_eq!(midranks(&[10.0, 20.0, 20.0, 5.0]), [2.0, 3.5, 3.5, 1.0]);
        assert_eq!(midranks(&[1.0, 1.0, 1.0]), [2.0, 2.0, 2.0]);
        assert_eq!(midranks(&[3.0, 1.0, 2.0]), [3.0, 1.0, 2.0]);
    }

    #[test]
    fn mann_whitney_u_counts_the_pairs_an_arm_value_exceeds_a_reference_value() {
        let reference = ramp(1.0, 9);
        let all_above = mann_whitney(&mut rng(1), &ramp(100.0, 9), &reference);
        assert_eq!(all_above.u, 81.0);
        let all_below = mann_whitney(&mut rng(1), &ramp(-100.0, 9), &reference);
        assert_eq!(all_below.u, 0.0);
        let identical = mann_whitney(&mut rng(1), &reference, &reference);
        assert_eq!(identical.u, 40.5, "ties count half");
        assert_eq!(identical.p, 1.0, "no permutation is less extreme than the null mean");
        assert!(all_above.p < 0.001, "a total separation is rare: {}", all_above.p);
        assert!(all_above.p > 0.0, "the add-one estimate is never zero");
    }

    #[test]
    fn mann_whitney_is_deterministic_under_the_seed() {
        let (a, b) = (ramp(1.0, 9), ramp(4.0, 9));
        let first = mann_whitney(&mut rng(7), &b, &a);
        let second = mann_whitney(&mut rng(7), &b, &a);
        assert_eq!((first.u, first.p), (second.u, second.p));
    }

    #[test]
    fn the_bootstrap_interval_is_deterministic_under_the_seed_and_brackets_the_delta() {
        let (reference, other) = (ramp(1000.0, 9), ramp(1500.0, 9));
        let first = bootstrap_interval(&mut rng(3), &reference, &other);
        let second = bootstrap_interval(&mut rng(3), &reference, &other);
        assert_eq!(first, second);
        assert!(first.lower <= 500.0 && 500.0 <= first.upper, "{first:?}");
        assert!(first.lower > 480.0 && first.upper < 520.0, "narrow for tight data: {first:?}");
        let other_seed = bootstrap_interval(&mut rng(4), &reference, &other);
        assert!(other_seed.lower <= 500.0 && 500.0 <= other_seed.upper);
    }

    #[test]
    fn identical_samples_give_a_degenerate_interval_at_zero() {
        let same = [5.0; 9];
        let interval = bootstrap_interval(&mut rng(1), &same, &same);
        assert_eq!(interval, Interval { lower: 0.0, upper: 0.0 });
    }

    #[test]
    fn the_verdict_needs_an_interval_excluding_zero_and_a_delta_beyond_the_floor() {
        let floor = Interval { lower: -4.0, upper: 3.0 };
        let shift =
            |delta, lower, upper| Shift { delta, interval: Interval { lower, upper }, floor };
        assert!(shift(500.0, 490.0, 510.0).distinguishable());
        assert!(shift(-500.0, -510.0, -490.0).distinguishable(), "a negative shift counts");
        assert!(!shift(500.0, -10.0, 510.0).distinguishable(), "the interval includes zero");
        assert!(!shift(3.5, 1.0, 6.0).distinguishable(), "excludes zero but not beyond the floor");
        assert!(
            !shift(4.0, 1.0, 6.0).distinguishable(),
            "equal to the larger bound is not beyond it"
        );
        assert!(shift(4.5, 1.0, 6.0).distinguishable(), "the floor's larger bound is |-4|");
        assert!(!shift(-3.5, -6.0, -1.0).distinguishable(), "|-3.5| is below 4");
        let wide_upper = Interval { lower: -1.0, upper: 9.0 };
        let upper_floor =
            Shift { delta: 8.0, interval: Interval { lower: 2.0, upper: 12.0 }, floor: wide_upper };
        assert!(!upper_floor.distinguishable(), "the larger absolute bound is the upper one");
        let zero_edge = Shift { delta: 8.0, interval: Interval { lower: 0.0, upper: 12.0 }, floor };
        assert!(!zero_edge.distinguishable(), "an interval touching zero does not exclude it");
        let negative_edge =
            Shift { delta: -8.0, interval: Interval { lower: -12.0, upper: 0.0 }, floor };
        assert!(!negative_edge.distinguishable());
    }

    #[test]
    fn fewer_than_nine_values_in_any_of_the_three_sets_is_insufficient() {
        let nine = ramp(1.0, 9);
        let eight = ramp(1.0, 8);
        for (reference, arm, control) in [
            (&eight, &nine, &nine),
            (&nine, &eight, &nine),
            (&nine, &nine, &eight),
            (&nine, &nine, &Vec::new()),
        ] {
            assert!(matches!(
                compare(&mut rng(1), reference, arm, control),
                Comparison::Insufficient
            ));
        }
        assert!(matches!(compare(&mut rng(1), &nine, &nine, &nine), Comparison::Compared { .. }));
    }

    #[test]
    fn a_clear_difference_is_distinguishable_and_a_shuffle_of_the_same_values_is_not() {
        let reference = ramp(1000.0, 9);
        let control: Vec<f64> = reference.iter().rev().copied().collect();
        let Comparison::Compared { shift, test } =
            compare(&mut rng(5), &reference, &ramp(1500.0, 9), &control)
        else {
            panic!("compared")
        };
        assert_eq!(shift.delta, 500.0);
        assert!(shift.distinguishable());
        assert_eq!(test.u, 81.0);
        let Comparison::Compared { shift, .. } =
            compare(&mut rng(5), &reference, &ramp(1001.0, 9), &control)
        else {
            panic!("compared")
        };
        assert_eq!(shift.delta, 1.0);
        assert!(!shift.distinguishable(), "a +1 shift inside a 9-wide spread is noise");
    }

    #[test]
    fn a_comparison_is_the_same_for_the_same_seed() {
        let reference = ramp(1000.0, 9);
        let arm = ramp(1003.0, 9);
        let run = |seed| match compare(&mut rng(seed), &reference, &arm, &reference) {
            Comparison::Compared { shift, test } => (shift.interval, shift.floor, test.u, test.p),
            Comparison::Insufficient => panic!("compared"),
        };
        assert_eq!(run(11), run(11));
    }
}
