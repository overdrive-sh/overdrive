//! DG-5 newtype (`.claude/rules/rust.md` § Newtype completeness):
//! `SocketCookie` — validating `new`, `get`, decimal `Display` / `FromStr`
//! round-trip; invalid input is a typed error. (The EVMOD cut removed the
//! drain cells and with them `CellGeneration`.)

use std::str::FromStr;

use proptest::prelude::*;

use super::super::{SocketCookie, SocketCookieError};

proptest! {
    /// Every non-zero cookie round-trips through `new` / `get` and decimal
    /// `Display` / `FromStr`.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER step 5"]
    fn socket_cookie_roundtrips(raw in 1u64..) {
        let cookie = SocketCookie::new(raw).expect("non-zero cookie");
        prop_assert_eq!(cookie.get(), raw);
        prop_assert_eq!(cookie.to_string(), raw.to_string());
        prop_assert_eq!(SocketCookie::from_str(&raw.to_string()), Ok(cookie));
    }

    /// Non-decimal input is refused as unparseable.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER step 5"]
    fn non_decimal_input_is_refused(s in "[^0-9]{1,8}|[0-9]{1,4}[^0-9]{1,4}|-[0-9]{1,4}") {
        prop_assert_eq!(SocketCookie::from_str(&s), Err(SocketCookieError::Unparseable));
    }
}

/// Cookie 0 is never a kernel cookie; overflow and the empty string are
/// unparseable.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER step 5"]
fn zero_cookie_overflow_and_empty_are_refused() {
    assert_eq!(SocketCookie::new(0), Err(SocketCookieError::Zero));
    assert_eq!(SocketCookie::from_str("0"), Err(SocketCookieError::Zero));
    assert_eq!(SocketCookie::from_str("18446744073709551616"), Err(SocketCookieError::Unparseable));
    assert_eq!(SocketCookie::from_str(""), Err(SocketCookieError::Unparseable));
}
