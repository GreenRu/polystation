//! Links the Windows resources (the program icon) into the executables.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
        let res = std::path::Path::new(&dir).join("assets").join("polystation.res");
        println!("cargo:rerun-if-changed=assets/polystation.res");
        println!("cargo:rustc-link-arg-bins={}", res.display());
    }
}
