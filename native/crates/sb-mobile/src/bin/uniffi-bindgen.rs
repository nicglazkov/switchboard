//! Writes the Swift or Kotlin side of this crate's API from a built library:
//! `cargo run -p sb-mobile --features bindgen --bin uniffi-bindgen -- generate --library <lib> --language swift
//! --out-dir <dir>`.

fn main() {
    uniffi::uniffi_bindgen_main()
}
