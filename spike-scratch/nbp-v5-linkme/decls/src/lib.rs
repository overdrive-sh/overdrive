//! Stand-in for `overdrive-bench-families`: three declarations in a separate dependency
//! crate. The bench main references NOTHING in here except a no-op `pub fn` call
//! is deliberately NOT made; see bench/benches/benchmarks.rs.
nbp_lib::declare!(D_A, "decl_a", "fam");
nbp_lib::declare!(D_B, "decl_b", "fam");
nbp_lib::declare!(D_C, "decl_c", "fam");

#[cfg(test)]
mod tests {
    #[test]
    fn registry_in_decls_unit_test_binary() {
        println!("decls unit-test binary sees {} entries: {:?}", nbp_lib::count(), nbp_lib::names());
        assert_eq!(nbp_lib::count(), 6);
    }
}
