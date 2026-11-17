//! Stand-in for `overdrive-bench`: owns the distributed slice and the declaring macro.
pub use linkme;
pub struct Decl { pub name: &'static str, pub family: &'static str }

#[linkme::distributed_slice]
pub static REGISTRY: [Decl] = [..];

pub fn count() -> usize { REGISTRY.len() }
pub fn names() -> Vec<&'static str> { let mut v: Vec<_> = REGISTRY.iter().map(|d| d.name).collect(); v.sort(); v }

/// Stand-in for #[benchmark]: emits one registration in the caller's crate.
#[macro_export]
macro_rules! declare {
    ($ident:ident, $name:literal, $family:literal) => {
        #[$crate::linkme::distributed_slice($crate::REGISTRY)]
        #[linkme(crate = $crate::linkme)]
        static $ident: $crate::Decl = $crate::Decl { name: $name, family: $family };
    };
}

// Declarations living in the library itself, exercised by its unit-test binary.
declare!(LIB_A, "lib_a", "fam");
declare!(LIB_B, "lib_b", "fam");
declare!(LIB_C, "lib_c", "fam");

#[cfg(test)]
mod tests {
    #[test]
    fn registry_collects_three() {
        println!("unit-test binary sees {} entries: {:?}", super::count(), super::names());
        assert_eq!(super::count(), 3);
    }
}
