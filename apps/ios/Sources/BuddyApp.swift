import SwiftUI

@main
struct BuddyApp: App {
    @State private var model = AppModel()
    @Environment(\.scenePhase) private var phase

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(model)
                // buddy://pair?d=… : the link behind the pairing QR (a camera scan, or a tap on the link).
                .onOpenURL { url in
                    guard url.scheme == "buddy" else { return }
                    Task { await model.pair(url.absoluteString) }
                }
                .onChange(of: phase) { _, now in
                    if now == .active { model.wake() }
                }
                #if DEBUG
                // BUDDY_DEBUG_PAIR=<buddy://pair?d=…>: pairs at launch, to try the app in the simulator (it has no
                // camera) without the system's «¿Abrir en Buddy?» question.
                .task {
                    if let link = ProcessInfo.processInfo.environment["BUDDY_DEBUG_PAIR"], model.machines.isEmpty { await model.pair(link) }
                }
                #endif
        }
    }
}

struct RootView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if model.machines.isEmpty {
            NavigationStack { PairView(first: true) }
        } else {
            TabView {
                Tab("Inicio", systemImage: "house") { NavigationStack { HomeView() } }
                Tab("Chats", systemImage: "bubble.left.and.bubble.right") { NavigationStack { ChatsView() } }
                Tab("Equipos", systemImage: "desktopcomputer") { NavigationStack { MachinesView() } }
            }
        }
    }
}
