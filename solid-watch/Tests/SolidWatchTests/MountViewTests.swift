import SwiftUI
import XCTest
@testable import SolidWatch

/// What MountView draws follows the session across bundle loads: a view
/// graph kept from the first bundle draws exactly what a fresh one draws
/// after another bundle is loaded into the same session. (Each mount numbers
/// its nodes from the same tags, so views keyed by tag alone kept showing
/// the first bundle.)
@MainActor
final class MountViewTests: XCTestCase {
    func bundle(_ name: String) throws -> (Data, String) {
        let dir = Fx.fixtures.appendingPathComponent("mutations/\(name)")
        let src = try Data(contentsOf: dir.appendingPathComponent("bundle.js"))
        let sha = String(try Fx.text("mutations/\(name)/bundle.js.sha256").split(separator: " ")[0])
        return (src, sha)
    }

    func digest(_ image: CGImage?) -> String? {
        guard let image, let data = image.dataProvider?.data as Data? else { return nil }
        return JSHost.sha256Hex(data)
    }

    func testDrawingFollowsBundleLoads() throws {
        let session = SolidWatchSession()
        session.makeMeasurer = { StubTextMeasurer() }
        let size = CGSize(width: 198, height: 242)
        session.setViewport(size)
        func view() -> some View { MountView(session: session).frame(width: size.width, height: size.height) }
        let kept = ImageRenderer(content: view())
        let loads: [(String, JSData)] = [
            ("basic", ["n": 1]),
            ("list", ["ids": [1, 2, 3]]),
            ("basic", ["n": 7]),
            ("list", ["ids": [4]]),
        ]
        var previous: String?
        for (name, state) in loads {
            let (src, sha) = try bundle(name)
            session.load(src, expectedSHA256: sha)
            session.setState(state)
            XCTAssertNil(session.error, name)
            // let the kept renderer see the published changes
            RunLoop.main.run(until: Date().addingTimeInterval(0.2))
            let shown = digest(kept.cgImage)
            XCTAssertNotNil(shown, name)
            XCTAssertEqual(shown, digest(ImageRenderer(content: view()).cgImage), "\(name): the kept view draws the loaded bundle")
            XCTAssertNotEqual(shown, previous, "\(name): the drawing changed")
            previous = shown
        }
    }
}
