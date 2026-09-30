import XCTest

final class NavigationTests: XCTestCase {
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
