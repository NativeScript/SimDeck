import XCTest

final class TouchInputUITests: XCTestCase {
    override func setUp() {
        super.setUp()
        continueAfterFailure = false
    }

    func testPinchEmitsTwoPointMultiTouch() {
        let app = XCUIApplication()
        app.launchArguments = ["--simdeck-touch-input-test"]
        app.launch()

        XCTAssertTrue(app.staticTexts["Touch Input Test"].waitForExistence(timeout: 5))

        app.pinch(withScale: 2.0, velocity: 1.0)

        let multiTouchEvent = app.staticTexts.matching(
            NSPredicate(format: "label CONTAINS %@", "multi")
        ).firstMatch
        XCTAssertTrue(multiTouchEvent.waitForExistence(timeout: 3))

        let singleTouchEvent = app.staticTexts.matching(
            NSPredicate(format: "label CONTAINS %@", "single")
        ).firstMatch
        XCTAssertFalse(singleTouchEvent.exists)
    }

    func testControllerPinchAgainstConnectedSimulator() throws {
        let environment = ProcessInfo.processInfo.environment
        guard let launchURL = environment["SIMDECK_E2E_URL"] ?? environment["TEST_RUNNER_SIMDECK_E2E_URL"] else {
            throw XCTSkip("Set SIMDECK_E2E_URL to run the controller-to-target simulator pinch test.")
        }

        let app = XCUIApplication()
        app.launchArguments = ["--simdeck-e2e-controller", "--simdeck-open-url=\(launchURL)"]
        app.launch()

        let streamSurface = app.otherElements["touch-input-surface"].firstMatch
        XCTAssertTrue(streamSurface.waitForExistence(timeout: 20), "Stream touch surface did not appear.")

        streamSurface.pinch(withScale: 2.0, velocity: 1.0)
        sleep(1)
    }
}
