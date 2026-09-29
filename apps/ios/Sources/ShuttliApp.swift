import SwiftUI

@main
struct ShuttliApp: App {
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var state = AppState()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(state)
        }
        .onChange(of: scenePhase) { phase in
            switch phase {
            case .active:
                state.enterForeground()
            case .background:
                state.enterBackground()
            case .inactive:
                break
            @unknown default:
                state.enterBackground()
            }
        }
    }
}

@MainActor
final class AppState: ObservableObject {
    @Published private(set) var draft: String?
    @Published private(set) var historyCount: UInt32 = 0
    private let session = MobileSession()

    func enterForeground() {
        _ = session.enterForeground()
        historyCount = session.historyCount()
    }

    func enterBackground() {
        session.enterBackground()
        draft = nil
    }

    func importText(_ text: String) {
        guard !text.isEmpty, text.utf8.count <= 1_048_576 else { return }
        draft = text
    }
}
