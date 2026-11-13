// Edge: an attribute macro on an INLINE module with no out-of-line children is
// accepted on stable (it just cannot reach a file module).
#[probe_macros::module_placement(guest = "probe")]
mod inline_only {
    #[test]
    fn t() {}
}
