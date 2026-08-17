import SwiftUI

/// Phase 0 exists to prove one thing: that a call made from Swift travels
/// through the compiled Rust core and comes back. `ping()` is the trivial
/// round-trip that proves it, generated from `x8086-ffi` by uniffi.
struct ContentView: View {
    private let pingResult = ping()

    var body: some View {
        Text("FFI round-trip: \(pingResult)")
            .padding()
            .accessibilityIdentifier("pingResultLabel")
    }
}

#Preview {
    ContentView()
}
