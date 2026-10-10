//! QPROJ-5 / DG-5 newtypes (`.claude/rules/rust.md` § Newtype completeness):
//! `SocketCookie` and `CellGeneration` — validating `new`, `get`, decimal
//! `Display` / `FromStr` round-trip; invalid input is a typed error.

use std::str::FromStr;

use proptest::prelude::*;

use super::super::{CellGeneration, CellGenerationError, SocketCookie, SocketCookieError};

proptest! {
    /// Every non-zero cookie round-trips through `new` / `get` and decimal
    /// `Display` / `FromStr`.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
    fn socket_cookie_roundtrips(raw in 1u64..) {
        let cookie = SocketCookie::new(raw).expect("non-zero cookie");
        prop_assert_eq!(cookie.get(), raw);
        prop_assert_eq!(cookie.to_string(), raw.to_string());
        prop_assert_eq!(SocketCookie::from_str(&raw.to_string()), Ok(cookie));
    }

    /// Every generation round-trips.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
    fn cell_generation_roundtrips(raw in any::<u64>()) {
        let generation = CellGeneration::new(raw).expect("every u64 is a generation");
        prop_assert_eq!(generation.get(), raw);
        prop_assert_eq!(generation.to_string(), raw.to_string());
        prop_assert_eq!(CellGeneration::from_str(&raw.to_string()), Ok(generation));
    }

    /// Non-decimal input is refused by both.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
    fn non_decimal_input_is_refused(s in "[^0-9]{1,8}|[0-9]{1,4}[^0-9]{1,4}|-[0-9]{1,4}") {
        prop_assert_eq!(SocketCookie::from_str(&s), Err(SocketCookieError::Unparseable));
        prop_assert_eq!(CellGeneration::from_str(&s), Err(CellGenerationError::Unparseable));
    }
}

/// Cookie 0 is never a kernel cookie; overflow is unparseable.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn zero_cookie_and_overflow_are_refused() {
    assert_eq!(SocketCookie::new(0), Err(SocketCookieError::Zero));
    assert_eq!(SocketCookie::from_str("0"), Err(SocketCookieError::Zero));
    assert_eq!(SocketCookie::from_str("18446744073709551616"), Err(SocketCookieError::Unparseable));
    assert_eq!(
        CellGeneration::from_str("18446744073709551616"),
        Err(CellGenerationError::Unparseable)
    );
    assert_eq!(CellGeneration::from_str(""), Err(CellGenerationError::Unparseable));
}
