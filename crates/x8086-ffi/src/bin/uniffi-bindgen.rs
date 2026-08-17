//! Generates Swift bindings from the compiled x8086-ffi library.
//! Invoked by `scripts/build-universal.sh`; not meant to be run by hand.

fn main() {
    uniffi::uniffi_bindgen_main()
}
