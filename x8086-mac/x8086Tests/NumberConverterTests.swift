import XCTest

@testable import x8086

final class NumberConverterTests: XCTestCase {
    func testParsesBareDecimal() {
        XCTAssertEqual(NumberConverter.parse("42"), 42)
        XCTAssertEqual(NumberConverter.parse("  42  "), 42)
    }

    func testParsesHexWithHSuffixCaseInsensitively() {
        XCTAssertEqual(NumberConverter.parse("2Ah"), 0x2A)
        XCTAssertEqual(NumberConverter.parse("FFH"), 0xFF)
    }

    func testParsesBinaryWithBSuffix() {
        XCTAssertEqual(NumberConverter.parse("101010b"), 42)
    }

    func testParsesOctalWithOOrQSuffix() {
        XCTAssertEqual(NumberConverter.parse("52o"), 42)
        XCTAssertEqual(NumberConverter.parse("52q"), 42)
    }

    func testRejectsEmptyOrUnrecognizedText() {
        XCTAssertNil(NumberConverter.parse(""))
        XCTAssertNil(NumberConverter.parse("   "))
        XCTAssertNil(NumberConverter.parse("not a number"))
        XCTAssertNil(NumberConverter.parse("XYZh"), "digits invalid even for the claimed radix must fail")
    }

    func testFormattingRoundTripsAcrossEveryBase() {
        XCTAssertEqual(NumberConverter.decimalText(42), "42")
        XCTAssertEqual(NumberConverter.hexText(42), "2A")
        XCTAssertEqual(NumberConverter.octalText(42), "52")
        XCTAssertEqual(NumberConverter.binaryText(42), "101010")
    }
}
