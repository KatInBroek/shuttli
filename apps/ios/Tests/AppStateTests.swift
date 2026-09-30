import XCTest
import UIKit
@testable import Shuttli

final class AppStateTests: XCTestCase {
    @MainActor
    func testImportFreezesDraftWithoutSendingOrChangingClipboard() {
        let state = AppState()
        UIPasteboard.general.string = "clipboard sentinel"
        state.importText("explicit draft")
        XCTAssertEqual(state.draft, "explicit draft")
        XCTAssertNil(state.imageDraft)
        XCTAssertTrue(state.transferRows.isEmpty)
        XCTAssertEqual(UIPasteboard.general.string, "clipboard sentinel")
        state.sendDraft()
        XCTAssertEqual(state.draft, "explicit draft")
        XCTAssertEqual(state.sendStatusKey, "no_send_targets")
        state.dismissDraft()
        XCTAssertNil(state.draft)
        XCTAssertNil(state.sendStatusKey)
        XCTAssertEqual(UIPasteboard.general.string, "clipboard sentinel")
    }

    @MainActor
    func testInvalidTextCannotReplaceAnAcceptedSnapshot() {
        let state = AppState()
        state.importText("accepted")
        for text in ["", "a\0b", String(repeating: "x", count: 1_048_577)] {
            state.importText(text)
            XCTAssertEqual(state.draft, "accepted")
        }
        XCTAssertTrue(state.transferRows.isEmpty)
    }

    @MainActor
    func testImageImportIsExplicitAndBackgroundDiscardsBothDraftKinds() {
        let state = AppState()
        let image = UIGraphicsImageRenderer(size: CGSize(width: 2, height: 2)).image { context in
            UIColor.blue.setFill(); context.fill(CGRect(x: 0, y: 0, width: 2, height: 2))
        }
        state.importText("previous text")
        state.importImage(image)
        XCTAssertNil(state.draft)
        XCTAssertNotNil(state.imageDraft)
        XCTAssertTrue(state.transferRows.isEmpty)
        state.enterBackground()
        XCTAssertNil(state.imageDraft)
        XCTAssertEqual(state.connectedPeerCount, 0)
        state.importText("next snapshot")
        state.enterBackground()
        XCTAssertNil(state.draft)
    }

    func testDevicePolicyDecodesLegacyContentDefaultsAndPreservesTypes() throws {
        let legacy = try JSONDecoder().decode(DeviceDirections.self, from: Data(#"{"send":false,"receive":true}"#.utf8))
        XCTAssertFalse(legacy.send); XCTAssertTrue(legacy.receive)
        XCTAssertTrue(legacy.text); XCTAssertTrue(legacy.image)
        let original = DeviceDirections(send: true, receive: false, text: false, image: true)
        let restored = try JSONDecoder().decode(DeviceDirections.self, from: JSONEncoder().encode(original))
        XCTAssertTrue(restored.send); XCTAssertFalse(restored.receive)
        XCTAssertFalse(restored.text); XCTAssertTrue(restored.image)
    }

    func testHistoryLimitsFailClosedAndPersistValidSettings() {
        let original = HistorySettingsStore.load()
        defer { _ = HistorySettingsStore.save(original) }
        XCTAssertFalse(HistorySettingsStore.save(HistorySettings(mode: "content", limit: -1)))
        XCTAssertFalse(HistorySettingsStore.save(HistorySettings(mode: "content", limit: 10_001)))
        XCTAssertFalse(HistorySettingsStore.save(HistorySettings(mode: "invalid", limit: 20)))
        XCTAssertTrue(HistorySettingsStore.save(HistorySettings(mode: "status", limit: 0)))
        XCTAssertEqual(HistorySettingsStore.load().mode, "status")
        XCTAssertEqual(HistorySettingsStore.load().limit, 0)
    }
}
