import XCTest

@testable import x8086

final class X8086Tests: XCTestCase {
    func testPingReturnsPong() {
        XCTAssertEqual(ping(), "pong")
    }
}
