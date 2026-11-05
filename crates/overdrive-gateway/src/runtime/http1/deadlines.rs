//! Pure deadline composition of the kernel relay (S-PIG-85; R-32, R-37).
//!
//! No function reads a clock: `now` is an input.

#![allow(
    clippy::redundant_pub_crate,
    reason = "the DESIGN pins these items `pub(crate)`: crate-private, reached by the exchange driver"
)]

use std::time::Instant;

use crate::ports::ReadDeadlines;
use crate::runtime::GatewayLimits;

/// The whole-request ceiling of a request whose first octet arrived at
/// `first_octet_at`.
pub(crate) fn request_deadline(first_octet_at: Instant, limits: &GatewayLimits) -> Instant {
    first_octet_at + limits.whole_request_deadline()
}

/// The `ReadDeadlines` the driver passes to `read_through` for each phase,
/// already reduced to `min(phase budget, request_deadline)`.
pub(crate) fn compose_read_deadlines(
    phase: ReadPhase,
    now: Instant,
    request_deadline: Instant,
    limits: &GatewayLimits,
) -> ReadDeadlines {
    match phase {
        ReadPhase::FirstRequestHead => {
            let by = request_deadline.min(now + limits.request_head_deadline());
            ReadDeadlines { first_octet: by, complete_within: None, complete_by: Some(by) }
        }
        ReadPhase::LaterRequestHead => ReadDeadlines {
            first_octet: now + limits.keep_alive_idle_deadline(),
            complete_within: Some(limits.request_head_deadline()),
            complete_by: None,
        },
        ReadPhase::FramingLine => ReadDeadlines {
            first_octet: now + limits.body_no_progress_deadline(),
            complete_within: Some(limits.body_no_progress_deadline()),
            complete_by: Some(request_deadline),
        },
        ReadPhase::ResponseHead { request_complete_at } => {
            let by = request_complete_at.map_or(request_deadline, |t0| {
                request_deadline.min(t0 + limits.response_head_deadline())
            });
            ReadDeadlines {
                first_octet: by,
                complete_within: Some(limits.response_head_deadline()),
                complete_by: Some(by),
            }
        }
    }
}

/// The read the deadlines are composed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadPhase {
    FirstRequestHead,
    LaterRequestHead,
    FramingLine,
    /// `request_complete_at`: `Some(t0)` once the upstream request is
    /// completely written (R-37), else `None`.
    ResponseHead {
        request_complete_at: Option<Instant>,
    },
}

#[cfg(test)]
mod laws {
    #![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

    use std::time::Duration;

    use proptest::prelude::*;

    use super::*;

    fn limits() -> GatewayLimits {
        GatewayLimits::first_slice()
    }

    /// An arbitrary but fixed time origin: only differences matter.
    fn origin() -> Instant {
        Instant::now()
    }

    fn after(base: Instant, millis: u64) -> Instant {
        base + Duration::from_millis(millis)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// S-PIG-85 (request deadline): a request that began at its first octet
        /// ends `whole_request_deadline` later, for every first-octet instant.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn a_request_ends_a_whole_request_deadline_after_its_first_octet(
            first_octet_ms in 0u64..=10_000_000,
        ) {
            let limits = limits();
            let first = after(origin(), first_octet_ms);
            prop_assert_eq!(
                request_deadline(first, &limits),
                first + limits.whole_request_deadline(),
            );
        }

        /// S-PIG-85 (first request head): the whole head, first-octet wait
        /// included, takes at most `request_head_deadline` from now (no 2x), and
        /// never outlives the request: `first_octet = complete_by =
        /// min(now + request_head_deadline, request_deadline)` and no
        /// `complete_within`.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn the_first_request_head_has_one_budget_reduced_by_the_request_deadline(
            now_ms in 0u64..=10_000_000,
            request_left_ms in 0u64..=240_000,
        ) {
            let limits = limits();
            let now = after(origin(), now_ms);
            let deadline = after(now, request_left_ms);
            let composed = compose_read_deadlines(ReadPhase::FirstRequestHead, now, deadline, &limits);
            let expected = deadline.min(now + limits.request_head_deadline());
            prop_assert_eq!(composed.first_octet, expected);
            prop_assert_eq!(composed.complete_by, Some(expected));
            prop_assert_eq!(composed.complete_within, None);
        }

        /// S-PIG-85 (later request head): an idle keep-alive connection waits
        /// `keep_alive_idle_deadline` for the first octet, then the head has
        /// `request_head_deadline` from that octet and no absolute ceiling (the
        /// first octet starts the request).
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn a_later_request_head_waits_idle_then_gets_the_head_budget_from_its_first_octet(
            now_ms in 0u64..=10_000_000,
        ) {
            let limits = limits();
            let now = after(origin(), now_ms);
            let deadline = request_deadline(now, &limits);
            let composed = compose_read_deadlines(ReadPhase::LaterRequestHead, now, deadline, &limits);
            prop_assert_eq!(composed.first_octet, now + limits.keep_alive_idle_deadline());
            prop_assert_eq!(composed.complete_within, Some(limits.request_head_deadline()));
            prop_assert_eq!(composed.complete_by, None);
        }

        /// S-PIG-85 (framing lines): a chunk-size, terminator or trailer line
        /// gets `body_no_progress_deadline` for its first octet and to complete
        /// once begun, and is always bounded by the request deadline.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn framing_lines_get_the_no_progress_budget_and_never_outlive_the_request(
            now_ms in 0u64..=10_000_000,
            request_left_ms in 0u64..=240_000,
        ) {
            let limits = limits();
            let now = after(origin(), now_ms);
            let deadline = after(now, request_left_ms);
            let composed = compose_read_deadlines(ReadPhase::FramingLine, now, deadline, &limits);
            prop_assert_eq!(composed.complete_within, Some(limits.body_no_progress_deadline()));
            prop_assert_eq!(composed.complete_by, Some(deadline));
            prop_assert!(
                composed.first_octet <= now + limits.body_no_progress_deadline(),
                "the first octet is awaited at most the no-progress budget",
            );
        }

        /// S-PIG-85 and R-37 (response head before the request is written): while
        /// the upstream request is not completely written the head is bounded by
        /// the request's own deadlines, not by `response_head_deadline`: the
        /// first octet and the ceiling are the request deadline, and an origin
        /// that answers early must finish its head within
        /// `response_head_deadline` of its first octet.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn a_response_head_read_before_the_request_is_written_follows_the_request_deadline(
            now_ms in 0u64..=10_000_000,
            request_left_ms in 0u64..=240_000,
        ) {
            let limits = limits();
            let now = after(origin(), now_ms);
            let deadline = after(now, request_left_ms);
            let composed = compose_read_deadlines(
                ReadPhase::ResponseHead { request_complete_at: None },
                now,
                deadline,
                &limits,
            );
            prop_assert_eq!(composed.first_octet, deadline);
            prop_assert_eq!(composed.complete_by, Some(deadline));
            prop_assert_eq!(composed.complete_within, Some(limits.response_head_deadline()));
        }

        /// S-PIG-85 and R-37 (response head from t0): once the upstream request
        /// is completely written at `t0` the head has one budget:
        /// `first_octet = complete_by = min(t0 + response_head_deadline,
        /// request_deadline)` and `complete_within = response_head_deadline`, so
        /// a silent origin costs at most `response_head_deadline` and never
        /// more than the request's remaining whole-request budget.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn a_response_head_read_from_t0_has_one_budget_reduced_by_the_request_deadline(
            t0_ms in 0u64..=10_000_000,
            since_t0_ms in 0u64..=10_000,
            request_left_ms in 0u64..=240_000,
        ) {
            let limits = limits();
            let t0 = after(origin(), t0_ms);
            let now = after(t0, since_t0_ms);
            let deadline = after(t0, request_left_ms);
            let composed = compose_read_deadlines(
                ReadPhase::ResponseHead { request_complete_at: Some(t0) },
                now,
                deadline,
                &limits,
            );
            let expected = deadline.min(t0 + limits.response_head_deadline());
            prop_assert_eq!(composed.first_octet, expected);
            prop_assert_eq!(composed.complete_by, Some(expected));
            prop_assert_eq!(composed.complete_within, Some(limits.response_head_deadline()));
        }
    }

    /// S-PIG-85 (the stated timelines): with the first-slice limits a request
    /// whose body upload completes at 118 of its 120 s has a response-head
    /// budget of 2 s, not 5 s; one that completes at 10 s has the full 5 s; and
    /// the response-head read before `t0` is still bounded by the request's
    /// remaining 120 s, never by the 5 s response-head budget.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn the_response_head_budget_is_the_smaller_of_its_own_and_the_remaining_request_time() {
        let limits = limits();
        let started = origin();
        let deadline = request_deadline(started, &limits);
        assert_eq!(
            deadline,
            started + Duration::from_secs(120),
            "first-slice whole-request budget"
        );
        let head_budget = Duration::from_secs(5);
        assert_eq!(limits.response_head_deadline(), head_budget);

        let late = started + Duration::from_secs(118);
        let composed = compose_read_deadlines(
            ReadPhase::ResponseHead { request_complete_at: Some(late) },
            late,
            deadline,
            &limits,
        );
        assert_eq!(composed.complete_by, Some(started + Duration::from_secs(120)), "2 s remain");

        let early = started + Duration::from_secs(10);
        let composed = compose_read_deadlines(
            ReadPhase::ResponseHead { request_complete_at: Some(early) },
            early,
            deadline,
            &limits,
        );
        assert_eq!(composed.complete_by, Some(started + Duration::from_secs(15)), "the full 5 s");

        let composed = compose_read_deadlines(
            ReadPhase::ResponseHead { request_complete_at: None },
            started + Duration::from_secs(40),
            deadline,
            &limits,
        );
        assert_eq!(composed.first_octet, deadline, "an unfinished upload is not cut at 5 s");
    }
}
