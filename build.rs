fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=resources");
        println!("cargo:rerun-if-changed=assets/bex.ico");
        embed_resource::compile("resources/bex.rc", embed_resource::NONE);
        embed_resource::compile_for_tests("resources/test.rc", embed_resource::NONE);
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
        println!("cargo:rustc-link-arg-bins=/PDBALTPATH:%_PDB%");
    }
}
