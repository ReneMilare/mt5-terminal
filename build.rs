//! Compiles the private indicator preset in when `preset/mod.rs` exists (see `src/studies.rs`).

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_preset)");
    println!("cargo::rerun-if-changed=preset");
    if std::path::Path::new("preset/mod.rs").exists() {
        println!("cargo::rustc-cfg=has_preset");
    }
}
