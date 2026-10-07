fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    println!("cargo:rustc-env=MIAOMINAL_BUILD_EPOCH_SECS={seconds}");
}
