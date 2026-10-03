import XCTest
@testable import Buddy

final class SourceIconsTests: XCTestCase {
    /// A link the model only wrote in its answer must not make Buddy contact that site by itself.
    func testOnlySitesTheCoreReportedMayAskForTheirIcon() {
        let reported = [ChatSource(title: "SENAMHI", url: "https://www.senamhi.gob.pe/pronostico")]
        let written = ChatSource(title: "x", url: "https://mzxw6ytboi.evil.example/")
        XCTAssertFalse(SourceIcons.mayFetch(written, reported: reported))
        XCTAssertFalse(SourceIcons.mayFetch(written, reported: []))
        // The reported page itself, and another page of the same site.
        XCTAssertTrue(SourceIcons.mayFetch(reported[0], reported: reported))
        XCTAssertTrue(SourceIcons.mayFetch(ChatSource(title: "", url: "https://senamhi.gob.pe/otra"), reported: reported))
        // A subdomain is another site: its name is where data would travel.
        XCTAssertFalse(SourceIcons.mayFetch(ChatSource(title: "", url: "https://datos.senamhi.gob.pe/"), reported: reported))
    }
}
