import Foundation

/// Pure 4-state FSM for island open/close logic.
/// No AppKit / AppState dependencies — communicates via `onTransition`.
///
/// The cursor drives it: moving onto the island opens it, moving off closes it
/// again, with no timer in between. Timers remain only as a safety net for an
/// island that opened while the cursor was somewhere else.
@MainActor
final class IslandStateMachine {

    enum State: Equatable {
        case hidden   // island invisible (notch size)
        case petit    // compact island (notch + ears)
        case home     // expanded, overview
        case hello   // expanded, greeting animation
    }

    private(set) var state: State = .hidden

    /// Fired on every transition: (from, to)
    var onTransition: ((State, State) -> Void)?

    /// home → petit delay (seconds). Override for debug.
    var homeToPetitDelay: TimeInterval = 15
    /// petit → hidden delay (seconds). Override for debug. `.infinity`: the compact island never hides by itself
    /// ("Mantener el notch").
    var petitToHiddenDelay: TimeInterval = 60
    /// hello → petit delay after greeting animation ends (no hover). ~0.6s syncs with canvas collapse.
    var greetAutoCollapseDelay: TimeInterval = 0.6
    /// hello → petit delay when mouse is hovering over the greeting.
    var greetHoverCollapseDelay: TimeInterval = 10
    /// Views the user is working in (alerts, chat): the idle timer never closes them.
    var pinned = false
    /// Set by the host before `mouseLeft()`: something is waiting for an answer (a
    /// permission request) or a drag is under way, so the cursor leaving must not
    /// close the island. Everything else closes the moment the cursor is gone.
    var holdOpen = false

    /// What leaving closes back to: where the island was before the cursor opened it.
    private enum Rest { case hidden, petit }
    private var returnTo: Rest = .petit

    private var petitHideWork: DispatchWorkItem?
    private var homeCollapseWork: DispatchWorkItem?
    private var greetCollapseWork: DispatchWorkItem?

    // MARK: – Inputs

    /// App launched or debug "launch greeting"
    func launch() {
        cancelTimers()
        transition(to: .hello)
    }

    /// Mouse entered the island notch area: hidden or compact opens straight to home.
    func mouseEntered() {
        switch state {
        case .hidden, .petit:
            cancelTimers()
            returnTo = (state == .hidden) ? .hidden : .petit
            transition(to: .home)
        case .home:
            homeCollapseWork?.cancel()
            homeCollapseWork = nil
        case .hello:
            // Mouse hovering during greeting — cancel short auto-collapse, extend to hover delay
            scheduleGreetCollapse(delay: greetHoverCollapseDelay)
        }
    }

    /// Mouse left the island notch area
    func mouseLeft() {
        switch state {
        case .hidden:
            break
        case .petit:
            schedulePetitHide()
        case .home:
            guard !holdOpen else { return }
            cancelTimers()
            transition(to: returnTo == .hidden ? .hidden : .petit)
        case .hello:
            // Interrupt greeting immediately → compact (overrides 10s auto-collapse)
            greetCollapseWork?.cancel(); greetCollapseWork = nil
            transition(to: .petit)
        }
    }

    /// The island opened, or settled into compact, with the cursor somewhere else
    /// (an event, a hotkey, a collapse button): arm the timer that eventually closes
    /// it, since no `mouseLeft()` is coming.
    func idle() {
        switch state {
        case .petit: schedulePetitHide()
        case .home:  scheduleHomeCollapse()
        default:     break
        }
    }

    /// Compact island clicked
    func click() {
        guard state == .petit else { return }
        cancelTimers()
        returnTo = .petit
        transition(to: .home)
    }

    /// Alert or explicit request: open straight to home.
    func forceHome() {
        cancelTimers()
        returnTo = .petit
        transition(to: .home)
    }

    /// Explicit close (OK button, Escape, an alert being answered).
    func forcePetit() {
        cancelTimers()
        transition(to: .petit)
    }

    /// The island was opened by something other than the FSM (alert, hotkey, menu
    /// item, file drop) and the caller has already opened it. Follow along silently:
    /// firing `onTransition` would rebuild the view that caller just chose.
    func adoptHome() {
        guard state == .hidden || state == .petit else { return }
        cancelTimers()
        returnTo = .petit
        state = .home
    }

    /// Greeting animation finished (called at T.end ≈ 4.60 s).
    /// Schedules auto-collapse. Does not override a longer hover timer already running.
    func greetComplete() {
        guard state == .hello else { return }
        // If mouse entered before this fires (hover timer already running), don't override it
        if greetCollapseWork == nil {
            scheduleGreetCollapse(delay: greetAutoCollapseDelay)
        }
    }

    private func scheduleGreetCollapse(delay: TimeInterval) {
        greetCollapseWork?.cancel()
        let item = DispatchWorkItem { [weak self] in
            guard let self, self.state == .hello else { return }
            self.transition(to: .petit)
        }
        greetCollapseWork = item
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: item)
    }

    /// Non-alert work event: show compact from hidden (HookServer reveal)
    func reveal() {
        guard state == .hidden else { return }
        cancelTimers()
        transition(to: .petit)
        schedulePetitHide()
    }

    // MARK: – Timers

    private func schedulePetitHide() {
        petitHideWork?.cancel()
        petitHideWork = nil
        guard petitToHiddenDelay.isFinite else { return }
        let item = DispatchWorkItem { [weak self] in
            guard let self, self.state == .petit else { return }
            self.transition(to: .hidden)
        }
        petitHideWork = item
        DispatchQueue.main.asyncAfter(deadline: .now() + petitToHiddenDelay, execute: item)
    }

    private func scheduleHomeCollapse() {
        homeCollapseWork?.cancel()
        if pinned { return }
        let item = DispatchWorkItem { [weak self] in
            guard let self, self.state == .home else { return }
            self.transition(to: .petit)
        }
        homeCollapseWork = item
        DispatchQueue.main.asyncAfter(deadline: .now() + homeToPetitDelay, execute: item)
    }

    func cancelTimers() {
        petitHideWork?.cancel();    petitHideWork = nil
        homeCollapseWork?.cancel(); homeCollapseWork = nil
        greetCollapseWork?.cancel(); greetCollapseWork = nil
    }

    private func transition(to new: State) {
        guard new != state else { return }
        let old = state
        state = new
        onTransition?(old, new)
    }

}
