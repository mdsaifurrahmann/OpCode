import XCTest

@testable import x8086

final class ListingExporterTests: XCTestCase {
    func testGeneratesOneRowPerCodeProducingLineWithItsAddressAndBytes() {
        let source = "MOV AX, 1\nMOV BX, 2\nHLT\n"
        let lineToAddress = [
            LineAddress(line: 1, address: 0),
            LineAddress(line: 2, address: 3),
            LineAddress(line: 3, address: 6),
        ]
        // MOV AX,1 -> B8 01 00; MOV BX,2 -> BB 02 00; HLT -> F4
        let memory: [UInt32: UInt8] = [
            0: 0xB8, 1: 0x01, 2: 0x00,
            3: 0xBB, 4: 0x02, 5: 0x00,
            6: 0xF4,
        ]

        let listing = ListingExporter.generate(
            source: source,
            lineToAddress: lineToAddress,
            machineCodeLength: 7,
            readMemory: { address, len in
                Data((0..<len).map { memory[address + $0] ?? 0 })
            }
        )

        XCTAssertTrue(listing.contains("0000  B8 01 00"), "listing: \(listing)")
        XCTAssertTrue(listing.contains("MOV AX, 1"), "listing: \(listing)")
        XCTAssertTrue(listing.contains("0003  BB 02 00"), "listing: \(listing)")
        XCTAssertTrue(listing.contains("MOV BX, 2"), "listing: \(listing)")
        XCTAssertTrue(listing.contains("0006  F4"), "listing: \(listing)")
        XCTAssertTrue(listing.contains("HLT"), "listing: \(listing)")
    }

    func testEmptyLineToAddressProducesJustTheHeader() {
        let listing = ListingExporter.generate(
            source: "",
            lineToAddress: [],
            machineCodeLength: 0,
            readMemory: { _, _ in Data() }
        )
        XCTAssertEqual(listing, "x8086 Listing\n\n")
    }
}
