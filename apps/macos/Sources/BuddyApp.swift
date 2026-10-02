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

    func applicationDidFinishLaunching(_ notification: Notification) {
        let tokens = DesignTokens.load()
        do {
            // An empty folder lets the core use ~/Library/Application Support/Buddy.
            let core = try BuddyCore(dataDir: "")
            let sprite = try core.sprite(id: "buddy-base")
            let pet = PetWindowController(core: core, sprite: sprite, tokens: tokens)
            pet.onClick = { [weak pet] in pet?.react("wave") }
            pet.show()
            pet.say(core.hello())
            self.core = core
            self.pet = pet
        } catch {
            let alert = NSAlert()
            alert.messageText = "Buddy no pudo arrancar"
            alert.informativeText = String(describing: error)
            alert.runModal()
            NSApp.terminate(nil)
        }
    }
}
