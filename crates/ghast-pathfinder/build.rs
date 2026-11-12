//! `cfg(ghast_modern)`: some modern version (26.1+) is enabled (one shared modern code path,
//! instantiated per version).

fn main() {
    println!("cargo::rustc-check-cfg=cfg(ghast_modern)");
    const MODERN: &[&str] = &["CARGO_FEATURE_V26_1", "CARGO_FEATURE_V26_2"];
    if MODERN.iter().any(|f| std::env::var_os(f).is_some()) {
        println!("cargo::rustc-cfg=ghast_modern");
    }
}
