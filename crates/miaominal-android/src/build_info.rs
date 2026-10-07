//! Compile-time description of the loaded native library.

#[derive(Debug, Clone, uniffi::Record)]
pub struct BuildInfo {
    pub abi_version: String,
    pub package_version: String,
    pub rust_target_os: String,
    pub rust_target_arch: String,
    pub profile: String,
    pub build_epoch_ms: u64,
}

impl BuildInfo {
    pub fn current() -> Self {
        let build_epoch_ms = env!("MIAOMINAL_BUILD_EPOCH_SECS")
            .parse::<u64>()
            .unwrap_or_default()
            * 1_000;
        Self {
            abi_version: crate::ABI_VERSION.to_string(),
            package_version: env!("CARGO_PKG_VERSION").to_string(),
            rust_target_os: std::env::consts::OS.to_string(),
            rust_target_arch: std::env::consts::ARCH.to_string(),
            profile: if cfg!(debug_assertions) {
                "debug".to_string()
            } else {
                "release".to_string()
            },
            build_epoch_ms,
        }
    }
}

#[uniffi::export]
pub fn android_bridge_info() -> BuildInfo {
    BuildInfo::current()
}
