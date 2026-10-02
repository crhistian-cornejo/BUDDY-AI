import AppKit
import UserNotifications

/// The macOS notification of PARLEY's automatic work ("Nuevo análisis · 11:00", "Resumen del día listo"): it reaches the
/// user even with the island hidden. Buttons: "Ver análisis" opens PARLEY's chat in the notch; "Resumen" opens the day's
/// digest page in the browser. Nothing is ever sent anywhere: it is the local notification centre. If the user has not
/// allowed notifications (or the build is not a proper app bundle), everything else still happens: the island and the
/// circle's bubble say it too.
@MainActor
final class Notifier: NSObject, UNUserNotificationCenterDelegate {
    static let shared = Notifier()

    private nonisolated static let category = "mika.parley"
    private nonisolated static let viewAction = "mika.parley.view"
    private nonisolated static let digestAction = "mika.parley.digest"
    private nonisolated static let digestKey = "digest"

    private var ready = false

    /// False while running tests or without an app bundle: the notification centre would trap there.
    private var available: Bool {
        Bundle.main.bundleIdentifier != nil && Bundle.main.bundleURL.pathExtension == "app"
            && ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] == nil
    }

    private func prepare() -> UNUserNotificationCenter? {
        guard available else { return nil }
        let center = UNUserNotificationCenter.current()
        if !ready {
            ready = true
            center.delegate = self
            let view = UNNotificationAction(identifier: Self.viewAction, title: "Ver análisis", options: [.foreground])
            let digest = UNNotificationAction(identifier: Self.digestAction, title: "Resumen del día", options: [.foreground])
            center.setNotificationCategories([
                UNNotificationCategory(identifier: Self.category, actions: [view, digest], intentIdentifiers: [], options: []),
            ])
            center.requestAuthorization(options: [.alert, .sound]) { _, _ in }
        }
        return center
    }

    /// `digestPath`: the page of today's digest, when it exists; the "Resumen del día" button opens it.
    func parley(title: String, body: String, digestPath: String?) {
        guard let center = prepare() else { return }
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default
        content.categoryIdentifier = Self.category
        if let digestPath { content.userInfo = [Self.digestKey: digestPath] }
        center.add(UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil))
    }

    // Shown even while MIKA is the active app: the whole point is not to miss it.
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                                            withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .sound])
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
                                            withCompletionHandler completionHandler: @escaping () -> Void) {
        let action = response.actionIdentifier
        let path = response.notification.request.content.userInfo[Self.digestKey] as? String
        completionHandler()
        Task { @MainActor in
            if action == Self.digestAction {
                if let path { Self.openDigest(path: path) } else { Self.openChat() }
            } else {
                Self.openChat()
            }
        }
    }

    /// PARLEY's chat in the notch, unless the user is in the middle of something there (the island open, a request
    /// waiting): then the bubble and the notification are enough.
    static func openChatIfFree() {
        let state = AppState.shared
        guard state.mode != .expanded, state.pendingApproval == nil else { return }
        openChat()
    }

    static func openChat() {
        (NSApp.delegate as? AppDelegate)?.islandController?.openChatInNotch(agent: Picks.agentID)
    }

    /// The digest page in the default browser. Only an `.html` file inside PARLEY's own `digest/` folder is opened.
    static func openDigest(path: String) {
        let url = URL(fileURLWithPath: path).standardizedFileURL
        guard url.pathExtension == "html", url.deletingLastPathComponent().lastPathComponent == "digest",
              FileManager.default.fileExists(atPath: url.path) else { return }
        NSWorkspace.shared.open(url)
    }
}
