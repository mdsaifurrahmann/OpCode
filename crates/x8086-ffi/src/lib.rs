//! Thin uniffi bridge to the emulator core. This crate contains no
//! business logic - only marshaling - so regenerating Swift bindings
//! never risks touching real behavior. `ping()` is a deliberately trivial
//! round-trip used to prove the whole pipeline (Rust -> staticlib ->
//! XCFramework -> Swift bindings -> SwiftUI call) before any real
//! emulator surface is exposed.

uniffi::setup_scaffolding!();

#[uniffi::export]
pub fn ping() -> String {
    "pong".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_returns_pong() {
        assert_eq!(ping(), "pong");
    }
}
