// `cfg(modern)`: any modern (26.x) version feature is on.
fn main() {
    println!("cargo::rustc-check-cfg=cfg(modern)");
    if std::env::var_os("CARGO_FEATURE_V26_1").is_some()
        || std::env::var_os("CARGO_FEATURE_V26_2").is_some()
    {
        println!("cargo::rustc-cfg=modern");
    }
}
