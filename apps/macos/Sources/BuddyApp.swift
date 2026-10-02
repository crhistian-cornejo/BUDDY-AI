import AppKit
import SwiftUI

/// Buddy lives outside the Dock (LSUIElement): the mascot is the app. Quit from its right-click menu.
@main
struct BuddyApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        Settings { EmptyView() }
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var core: BuddyCore?
    private var pet: PetWindowController?
    private var chat: ChatController?
    private var chatWindows: ChatWindows?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let tokens = DesignTokens.load()
        do {
            // An empty folder lets the core use ~/Library/Application Support/Buddy.
            let core = try BuddyCore(dataDir: "")
            let sprite = try core.sprite(id: "buddy-base")
            let pet = PetWindowController(core: core, sprite: sprite, tokens: tokens)
            let chat = ChatController(core: core)
            let windows = ChatWindows(chat: chat, tokens: tokens, pet: { [weak pet] in pet?.frame ?? .zero })
            pet.onClick = { [weak windows] in windows?.toggle() }
            windows.onOpenChange = { [weak pet] open in pet?.holdStill = open }
            // Chat events draw the chat; mascot events animate Buddy.
            core.subscribe(listener: CoreEvents { [weak chat, weak pet] event in
                chat?.handle(event)
                if case let .mascotState(state) = event { pet?.showMascotState(state) }
            })
            pet.show()
            pet.say(core.hello())
            self.core = core
            self.pet = pet
            self.chat = chat
            self.chatWindows = windows
            #if DEBUG
            // BUDDY_DEBUG_PROMPT="…": opens the composer and sends it, to try the whole flow from a terminal.
            if let prompt = ProcessInfo.processInfo.environment["BUDDY_DEBUG_PROMPT"] {
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                    windows.open()
                    chat.draft = prompt
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
}
