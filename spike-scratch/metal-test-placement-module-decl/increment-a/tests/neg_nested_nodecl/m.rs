use probe_macros::{overdrive_test, shared_guest};

shared_guest!(guest = "probe");

mod inner {
    use probe_macros::overdrive_test;

    #[overdrive_test]
    fn nested_without_glob() {}
}
