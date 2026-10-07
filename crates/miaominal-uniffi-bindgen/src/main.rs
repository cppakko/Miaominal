//! Host-side binding generator for the Android bridge.
//!
//! This binary intentionally lives outside `miaominal-android` so that the CLI and
//! code-generation dependencies are never linked into the shipped native library.

fn main() {
    uniffi::uniffi_bindgen_main()
}
