//! The log-linear histogram a series metric is stored as above 1,000 values (DDD-5, DDD-6).
//!
//! Bucket 0 is `[0, 1)`. A value `v` of at least 1 falls in bucket `1 + 32e + s`, with `e`
//! the unbiased binary exponent read from the `f64` bits and `s` the top five bits of the
//! fraction; a value at or above 2^64 clamps to bucket 2048. The stored form is sparse
//! `[bucket, count]` pairs with the exact `count`, `min` and `max`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The highest bucket index (a value at or above 2^64 lands here).
pub const LAST_BUCKET: u32 = 2048;

/// A series of more values than this is stored as a histogram, not raw.
pub const RAW_VALUE_LIMIT: usize = 1000;

const FRACTION_BITS: u32 = 52;
const EXPONENT_BIAS: u64 = 1023;
const SUB_BUCKET_BITS: u32 = 5;
const SUB_BUCKETS: u32 = 1 << SUB_BUCKET_BITS;
const OCTAVES: u64 = 64;

/// A series that cannot be stored: nothing to store, or a value DDD-5 refuses.
#[derive(Debug, PartialEq, thiserror::Error)]
pub enum NotStorable {
    #[error("a series needs at least one value")]
    Empty,
    #[error("value {0} is negative, NaN or infinite")]
    Value(f64),
}

/// A value a series may hold: finite and not negative.
pub fn storable(value: f64) -> Result<f64, NotStorable> {
    if value.is_finite() && value >= 0.0 { Ok(value) } else { Err(NotStorable::Value(value)) }
}

/// The bucket a value falls in (DDD-5 step 1), read from the bits, never from `log2`.
pub fn bucket_of(value: f64) -> u32 {
    if value < 1.0 {
        return 0;
    }
    let bits = value.to_bits();
    let exponent = ((bits >> FRACTION_BITS) & 0x7ff) - EXPONENT_BIAS;
    if exponent >= OCTAVES {
        return LAST_BUCKET;
    }
    let sub_bucket = (bits >> (FRACTION_BITS - SUB_BUCKET_BITS)) & u64::from(SUB_BUCKETS - 1);
    // `exponent` is below 64 and `sub_bucket` below 32, so both fit a `u32`.
    1 + SUB_BUCKETS * u32::try_from(exponent).unwrap_or(0) + u32::try_from(sub_bucket).unwrap_or(0)
}

/// A series stored as buckets: every value of it, none dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Histogram {
    pub count: u64,
    pub min: f64,
    pub max: f64,
    pub buckets: Vec<(u32, u64)>,
}

impl Histogram {
    /// Bin every value of `values`, which must be non-empty, finite and not negative.
    pub fn from_values(values: &[f64]) -> Result<Self, NotStorable> {
        let mut occupied = BTreeMap::<u32, u64>::new();
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for &value in values {
            storable(value)?;
            *occupied.entry(bucket_of(value)).or_default() += 1;
            min = min.min(value);
            max = max.max(value);
        }
        if values.is_empty() {
            return Err(NotStorable::Empty);
        }
        Ok(Self { count: values.len() as u64, min, max, buckets: occupied.into_iter().collect() })
    }

    /// The first rule this histogram breaks, in the DDD-6 order, or `None` when it is well formed.
    pub fn shape_error(&self) -> Option<String> {
        self.bucket_error().or_else(|| self.count_error()).or_else(|| self.extremes_error())
    }

    fn bucket_error(&self) -> Option<String> {
        let total = self.buckets.iter().try_fold(0u64, |sum, (_, count)| sum.checked_add(*count));
        if total != Some(self.count) {
            return Some(format!("buckets do not sum to count {}", self.count));
        }
        if let Some((index, _)) = self.buckets.iter().find(|(index, _)| *index > LAST_BUCKET) {
            return Some(format!("buckets: index {index} exceeds {LAST_BUCKET}"));
        }
        if self.buckets.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Some("buckets: indices must ascend and be distinct".to_owned());
        }
        self.buckets
            .iter()
            .find(|(_, count)| *count == 0)
            .map(|(index, _)| format!("buckets: bucket {index} holds no values"))
    }

    fn count_error(&self) -> Option<String> {
        (self.count <= RAW_VALUE_LIMIT as u64).then(|| {
            format!(
                "histogram of {} values: a histogram holds more than {RAW_VALUE_LIMIT}",
                self.count
            )
        })
    }

    fn extremes_error(&self) -> Option<String> {
        if self.min > self.max {
            return Some(format!("min {} exceeds max {}", self.min, self.max));
        }
        let (lowest, highest) = (self.buckets.first()?.0, self.buckets.last()?.0);
        if self.min < 0.0 || bucket_of(self.min) != lowest {
            return Some(format!("min {} lies outside its lowest bucket {lowest}", self.min));
        }
        (bucket_of(self.max) != highest)
            .then(|| format!("max {} lies outside its highest bucket {highest}", self.max))
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "tests assert their own preconditions and compare exact stored floats"
)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// The `[lower, upper)` range of a bucket from DDD-5, as an independent oracle.
    fn range(bucket: u32) -> (f64, f64) {
        if bucket == 0 {
            return (0.0, 1.0);
        }
        let (e, s) = ((bucket - 1) / 32, (bucket - 1) % 32);
        let base = 2f64.powi(i32::try_from(e).expect("exponent"));
        (base * (1.0 + f64::from(s) / 32.0), base * (1.0 + f64::from(s + 1) / 32.0))
    }

    #[test]
    fn bucket_boundaries_follow_the_binary_exponent() {
        for (value, bucket) in [
            (0.0, 0),
            (0.999_999, 0),
            (1.0, 1),
            (1.031_249, 1),
            (1.031_25, 2),
            (1.968_75, 32),
            (2.0, 33),
            (2000.0, 351),
            (9_223_372_036_854_775_808.0, 2017),
            (18_446_744_073_709_549_568.0, 2048),
            (18_446_744_073_709_551_616.0, 2048),
            (1e300, 2048),
        ] {
            assert_eq!(bucket_of(value), bucket, "bucket of {value}");
        }
        assert_eq!(bucket_of(f64::from_bits(1)), 0, "a subnormal falls in bucket 0");
    }

    #[test]
    fn values_the_protocol_refuses_are_not_storable() {
        for bad in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(matches!(Histogram::from_values(&[1.0, bad]), Err(NotStorable::Value(_))));
        }
        assert_eq!(Histogram::from_values(&[]), Err(NotStorable::Empty));
        assert!(Histogram::from_values(&[0.0]).is_ok(), "zero is a value");
    }

    fn wellformed(count: u64, min: f64, max: f64, buckets: Vec<(u32, u64)>) -> Histogram {
        Histogram { count, min, max, buckets }
    }

    #[test]
    fn each_shape_rule_is_enforced_in_the_ddd_6_order() {
        let good = wellformed(2000, 1.0, 2000.0, vec![(1, 1000), (351, 1000)]);
        assert_eq!(good.shape_error(), None);
        let cases = [
            (wellformed(2001, 1.0, 2000.0, vec![(1, 1000), (351, 1000)]), "sum"),
            (wellformed(2000, 1.0, 2000.0, vec![(1, 1000), (2049, 1000)]), "exceeds"),
            (wellformed(2000, 1.0, 2000.0, vec![(1, 500), (1, 500), (351, 1000)]), "ascend"),
            (wellformed(2000, 1.0, 2000.0, vec![(1, 1000), (100, 0), (351, 1000)]), "no values"),
            (wellformed(1000, 1.0, 2000.0, vec![(1, 500), (351, 500)]), "more than"),
            (wellformed(2000, 2010.0, 1990.0, vec![(351, 2000)]), "exceeds max"),
            (wellformed(2000, 0.5, 2000.0, vec![(1, 1000), (351, 1000)]), "lowest"),
            (wellformed(2000, 1.5, 2000.0, vec![(1, 1000), (351, 1000)]), "lowest"),
            (wellformed(2000, 1.0, 3000.0, vec![(1, 1000), (351, 1000)]), "highest"),
            (wellformed(2000, 1.0, 1000.0, vec![(1, 1000), (351, 1000)]), "highest"),
            (wellformed(2000, -1.0, 2000.0, vec![(0, 1000), (351, 1000)]), "lowest"),
        ];
        for (histogram, needle) in cases {
            let error = histogram.shape_error().unwrap_or_default();
            assert!(error.contains(needle), "expected `{needle}`, got `{error}`");
        }
    }

    #[test]
    fn the_boundaries_of_each_rule_are_accepted() {
        let at_top = wellformed(1001, 1.0, 1e30, vec![(1, 1), (2048, 1000)]);
        assert_eq!(at_top.shape_error(), None, "index 2048 and an unbounded last bucket");
        let at_edges = wellformed(1001, 0.0, 1.0, vec![(0, 1000), (1, 1)]);
        assert_eq!(at_edges.shape_error(), None, "min 0 in bucket 0, max 1 in bucket 1");
        let equal = wellformed(1001, 5.0, 5.0, vec![(73, 1001)]);
        assert_eq!(equal.shape_error(), None, "min equal to max");
        let overflow = wellformed(1, 1.0, 1.0, vec![(1, u64::MAX), (2, 2)]);
        assert!(overflow.shape_error().is_some(), "a bucket total that overflows is refused");
    }

    proptest! {
        #[test]
        fn encoding_keeps_every_value_in_its_bucket(
            values in proptest::collection::vec(
                prop_oneof![0.0f64..1.0, 1.0f64..1e6, 1.0f64..1.8e19], 1..1500),
        ) {
            let histogram = Histogram::from_values(&values).expect("storable");
            prop_assert_eq!(histogram.count, values.len() as u64);
            prop_assert_eq!(histogram.buckets.iter().map(|(_, c)| c).sum::<u64>(), histogram.count);
            prop_assert!(histogram.buckets.windows(2).all(|p| p[0].0 < p[1].0));
            prop_assert!(histogram.buckets.iter().all(|(b, c)| *b <= LAST_BUCKET && *c >= 1));
            prop_assert_eq!(histogram.min, values.iter().copied().fold(f64::INFINITY, f64::min));
            prop_assert_eq!(histogram.max, values.iter().copied().fold(f64::NEG_INFINITY, f64::max));
            for value in &values {
                let (lower, upper) = range(bucket_of(*value));
                prop_assert!(*value >= lower && *value < upper, "{value} outside [{lower}, {upper})");
            }
            let wellformed_error = histogram.shape_error();
            prop_assert_eq!(wellformed_error.is_none(), values.len() > RAW_VALUE_LIMIT);
        }

        #[test]
        fn the_bucket_never_decreases_as_the_value_grows(a in 0.0f64..1e30, b in 0.0f64..1e30) {
            let (low, high) = if a <= b { (a, b) } else { (b, a) };
            prop_assert!(bucket_of(low) <= bucket_of(high));
        }
    }
}
