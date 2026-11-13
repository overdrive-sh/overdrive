mod neg_attr_nested_file_mod {
    #[probe_macros::module_placement(guest = "probe")]
    mod outer {
        mod y;
    }
}
