import Foundation

/// Produces a classic assembler-listing text: each code-producing source
/// line paired with the address and machine-code bytes it assembled to.
/// A pure function (memory access is injected) so it's testable without
/// a live `Emulator` - reads bytes back from wherever the caller's
/// program is loaded rather than needing raw bytes threaded across FFI,
/// since by the time anyone exports a listing, the program is already
/// assembled and sitting in memory there.
enum ListingExporter {
    static func generate(
        source: String,
        lineToAddress: [LineAddress],
        machineCodeLength: UInt32,
        readMemory: (UInt32, UInt32) -> Data
    ) -> String {
        let sortedMappings = lineToAddress.sorted { $0.line < $1.line }
        let sourceLines = source.components(separatedBy: "\n")

        var output = "OpCode Listing\n\n"
        for (index, mapping) in sortedMappings.enumerated() {
            let nextAddress =
                index + 1 < sortedMappings.count ? sortedMappings[index + 1].address : machineCodeLength
            let byteLength = nextAddress > mapping.address ? nextAddress - mapping.address : 0
            let bytes = [UInt8](readMemory(mapping.address, byteLength))
            let hexBytes = bytes.map { String(format: "%02X", $0) }.joined(separator: " ")

            let lineIndex = Int(mapping.line) - 1
            let sourceLine = sourceLines.indices.contains(lineIndex) ? sourceLines[lineIndex] : ""

            let addressColumn = String(format: "%04X", mapping.address)
            let bytesColumn = hexBytes.padding(toLength: max(hexBytes.count, 24), withPad: " ", startingAt: 0)
            output += "\(addressColumn)  \(bytesColumn)  \(sourceLine)\n"
        }
        return output
    }
}
