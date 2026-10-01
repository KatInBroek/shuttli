import XCTest

final class NavigationTests: XCTestCase {
    func testWindowUsesTheFullScreenAspectRatio() {
        let app = XCUIApplication()
        app.launch()
        XCTAssertTrue(app.tabBars.firstMatch.waitForExistence(timeout: 10))
        let window = app.windows.firstMatch.frame
        let screen = XCUIScreen.main.screenshot().image.size
        XCTAssertGreaterThan(window.width, 0)
        XCTAssertGreaterThan(window.height, 0)
        XCTAssertEqual(window.height / window.width, screen.height / screen.width, accuracy: 0.03,
                       "The app must use the device screen, without legacy letterboxing")
    }

    func testDutchHistoryFiltersStayOnOneLine() {
        let app = XCUIApplication()
        app.launchArguments = ["-AppleLanguages", "(nl)", "-AppleLocale", "nl_NL", "-ui-language", "nl"]
        app.launch()
        let all = app.buttons["Alles"]
        let sent = app.buttons["Verzonden vanaf deze telefoon"]
        XCTAssertTrue(all.waitForExistence(timeout: 10))
        XCTAssertTrue(sent.exists)
        XCTAssertEqual(all.frame.height, sent.frame.height, accuracy: 2,
                       "Long localized filters should scroll horizontally rather than wrap")
    }

    func testThreeTabsAndExplicitSendPanelNavigation() {
        let app = XCUIApplication()
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "-ui-language", "en"]
        app.launch()
        XCTAssertTrue(app.tabBars.buttons["Home"].waitForExistence(timeout: 10))
        XCTAssertEqual(app.tabBars.buttons.count, 3)
        XCTAssertTrue(app.staticTexts["Send clipboard"].exists)
        app.tabBars.buttons["Devices"].tap()
        XCTAssertFalse(app.staticTexts["Send clipboard"].exists)
        app.tabBars.buttons["Settings"].tap()
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label CONTAINS[c] %@", "This phone")).firstMatch.exists)
        XCTAssertFalse(app.staticTexts["Send clipboard"].exists)
        app.tabBars.buttons["Home"].tap()
        XCTAssertTrue(app.staticTexts["Send clipboard"].exists)
    }
}
