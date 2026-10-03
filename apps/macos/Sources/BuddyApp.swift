import AppKit
import SwiftUI

/// Buddy lives outside the Dock (LSUIElement): the mascot is the app. Quit from its right-click menu.
@main
struct BuddyApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        Settings { SettingsView() }
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var core: BuddyCore?
    private var pet: PetWindowController?
    private var chat: ChatController?
    private var chatWindows: ChatWindows?
    private var notch: NotchController?
    private var briefingTimer: Timer?
    private var shortcutMonitor: Any?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let tokens = DesignTokens.load()
        do {
            // An empty folder lets the core use ~/Library/Application Support/Buddy.
            let core = try BuddyCore(dataDir: "")
            AppServices.core = core
            let sprite = try core.sprite(id: "buddy-base")
            Avatar.configure(sprite: sprite)
            let pet = PetWindowController(core: core, sprite: sprite, tokens: tokens)
            let chat = ChatController(core: core)
            let windows = ChatWindows(chat: chat, pet: { [weak pet] in pet?.frame ?? .zero })
            pet.onClick = { [weak windows] in windows?.toggle() }
            pet.onHistory = { [weak windows] in windows?.showHistory() }
            windows.onOpenChange = { [weak pet] open in pet?.holdStill = open }
            let notch = NotchController(core: core)
            notch.onOpenChat = { [weak chat, weak windows] id in
                chat?.open(id)
                windows?.open()
            }
            // Files dropped on the notch: a new chat with them attached.
            notch.onGiveFiles = { [weak chat, weak windows] files in
                chat?.newChat()
                chat?.attach(files)
                windows?.open()
            }
            // Chat events draw the chat, session events the notch; mascot events animate Buddy.
            core.subscribe(listener: CoreEvents { [weak chat, weak pet, weak notch] event in
                chat?.handle(event)
                notch?.handle(event)
                if case let .mascotState(state) = event { pet?.showMascotState(state) }
                // A new «mensajito»: Buddy says the first line; the notch keeps the list.
                if case let .briefingReady(count, headline) = event {
                    pet?.say(Self.short(headline) + (count > 1 ? " (+\(count - 1))" : ""), seconds: 8)
                    NotificationCenter.default.post(name: .buddyBriefingReady, object: nil)
                }
            })
            notch.start()
            // Claude Code and Codex hooks: the relay ships in the bundle; the core copies it to a stable place.
            let relay = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/buddy-hook").path
            try? core.startSessions(relayPath: FileManager.default.fileExists(atPath: relay) ? relay : "")
            self.notch = notch
            pet.show()
            pet.say(core.hello())
            self.core = core
            self.pet = pet
            self.chat = chat
            self.chatWindows = windows
            // Local shortcuts only, while a Buddy window has keyboard focus (no global keyboard hook).
            shortcutMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                guard let self else { return event }
                let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
                let key = event.charactersIgnoringModifiers?.lowercased()
                if modifiers == [.command, .shift], key == "p" {
                    self.pet?.toggleWander()
                    return nil
                }
                guard modifiers == .command else { return event }
                switch key {
                case "f": self.chatWindows?.showHistory()
                case "n": self.chat?.newChat(); self.chatWindows?.open()
                case "w":
                    guard event.window is NSPanel else { return event }
                    self.chatWindows?.close()
                case ",": SettingsWindow.show()
                case "q": NSApp.terminate(nil)
                default: return event
                }
                return nil
            }
            // Today's runs (8, 13, 19 h): once a little after launch, then on the hour. The core skips what already ran.
            DispatchQueue.main.asyncAfter(deadline: .now() + 20) { core.briefingTick() }
            let timer = Timer(timeInterval: 3600, repeats: true) { _ in core.briefingTick() }
            timer.tolerance = 300
            RunLoop.main.add(timer, forMode: .common)
            briefingTimer = timer
            #if DEBUG
            // BUDDY_DEBUG_PROMPT="…": opens the composer and sends it, to try the whole flow from a terminal.
            if ProcessInfo.processInfo.environment["BUDDY_DEBUG_SETTINGS"] != nil {
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { SettingsWindow.show() }
            }
            if ProcessInfo.processInfo.environment["BUDDY_DEBUG_HISTORY"] != nil {
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { windows.showHistory() }
            }
            // BUDDY_DEBUG_DRAFT="…": opens the composer with that text, unsent (to look at a long draft).
            if let draft = ProcessInfo.processInfo.environment["BUDDY_DEBUG_DRAFT"] {
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                    windows.open()
                    chat.draft = draft
                }
            }
            if let prompt = ProcessInfo.processInfo.environment["BUDDY_DEBUG_PROMPT"] {
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                    windows.open()
                    chat.draft = prompt
                    if let file = ProcessInfo.processInfo.environment["BUDDY_DEBUG_ATTACH"] { chat.attach([URL(fileURLWithPath: file)]) }
                    chat.send()
                }
            }
            #endif
        } catch {
            let alert = NSAlert()
            alert.messageText = "Buddy no pudo arrancar"
            alert.informativeText = String(describing: error)
            alert.runModal()
            NSApp.terminate(nil)
        }
    }

    /// One line for Buddy's bubble (it does not wrap).
    private static func short(_ text: String, limit: Int = 72) -> String {
        text.count <= limit ? text : String(text.prefix(limit - 1)).trimmingCharacters(in: .whitespaces) + "…"
    }
}
