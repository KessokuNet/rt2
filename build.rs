fn main() {
    println!("cargo::rerun-if-changed=src/shim/variadic.c");
    cc::Build::new()
        .file("src/shim/variadic.c")
        .std("c23")
        .warnings(true)
        .compile("rt2_variadic");
}
