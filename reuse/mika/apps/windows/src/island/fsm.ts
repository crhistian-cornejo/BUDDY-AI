// Island open/close FSM — port of IslandStateMachine.swift.
// No DOM, no Tauri: it only reports transitions.
//
// The cursor drives it: moving onto the island opens it, moving off closes it
// again, with no timer in between. Timers remain only as a safety net for an
// island that opened while the cursor was somewhere else.

export type FsmState = "hidden" | "petit" | "home" | "hello";

export class IslandStateMachine {
  state: FsmState = "hidden";
  private petMode = false;

  /** Mascot mode has no notch, including during launch, alerts and hover. */
  setPetMode(on: boolean) {
    this.petMode = on;
    if (on) this.forceHidden();
  }

  onTransition: ((from: FsmState, to: FsmState) => void) | null = null;

  /** home → petit delay, seconds. */
  homeToPetitDelay = 15;
  /** petit → hidden delay, seconds. */
  petitToHiddenDelay = 60;
  /** hello → petit once the greeting animation ends (no hover). */
  greetAutoCollapseDelay = 0.6;
  /** hello → petit while the mouse hovers the greeting. */
  greetHoverCollapseDelay = 10;
  /** Views the user is working in (chat, alerts): the idle timer never closes them. */
  pinned = false;
  /**
   * The host sets this before mouseLeft(): something is waiting for an answer (a
   * permission request) or a drag is under way, so the cursor leaving must not
   * close the island. Everything else closes the moment the cursor is gone.
   */
  holdOpen = false;

  /** What leaving closes back to: where the island was before the cursor opened it. */
  private returnTo: "hidden" | "petit" = "petit";

  private petitHide: number | null = null;
  private homeCollapse: number | null = null;
  private greetCollapse: number | null = null;

  // ── Inputs ──────────────────────────────────────────────────────────────────

  launch() {
    this.cancelTimers();
    this.transition("hello");
  }

  mouseEntered() {
    switch (this.state) {
      case "hidden":
      case "petit":
        this.cancelTimers();
        this.returnTo = this.state;
        this.transition("home");
        break;
      case "home":
        this.clear("homeCollapse");
        break;
      case "hello":
        this.scheduleGreetCollapse(this.greetHoverCollapseDelay);
        break;
    }
  }

  mouseLeft() {
    switch (this.state) {
      case "hidden":
        break;
      case "petit":
        this.schedulePetitHide();
        break;
      case "home":
        if (this.holdOpen) break;
        this.cancelTimers();
        this.transition(this.returnTo);
        break;
      case "hello":
        this.clear("greetCollapse");
        this.transition("petit");
        break;
    }
  }

  /**
   * The island opened, or settled into compact, with the cursor somewhere else
   * (an event, a hotkey, a collapse button): arm the timer that eventually
   * closes it, since no mouseLeft() is coming.
   */
  idle() {
    if (this.state === "petit") this.schedulePetitHide();
    else if (this.state === "home") this.scheduleHomeCollapse();
  }

  click() {
    if (this.state !== "petit") return;
    this.cancelTimers();
    this.returnTo = "petit";
    this.transition("home");
  }

  /** Greeting animation finished (T.end). Doesn't override a running hover timer. */
  greetComplete() {
    if (this.state !== "hello") return;
    if (this.greetCollapse == null) this.scheduleGreetCollapse(this.greetAutoCollapseDelay);
  }

  /** Non-alert work event: show compact from hidden. */
  reveal() {
    if (this.state !== "hidden") return;
    this.cancelTimers();
    this.transition("petit");
    this.schedulePetitHide();
  }

  /** Alert or explicit request: open straight to expanded. */
  forceHome() {
    this.cancelTimers();
    this.returnTo = "petit";
    this.transition("home");
  }

  /// Explicit close (OK button, Escape, an alert being answered).
  forcePetit() {
    this.cancelTimers();
    this.transition("petit");
  }

  forceHidden() {
    this.cancelTimers();
    this.transition("hidden");
  }

  // ── Timers ──────────────────────────────────────────────────────────────────

  private schedulePetitHide() {
    this.clear("petitHide");
    // "Keep the notch": the compact bar never hides by itself.
    if (!Number.isFinite(this.petitToHiddenDelay)) return;
    this.petitHide = window.setTimeout(() => {
      this.petitHide = null;
      if (this.state === "petit") this.transition("hidden");
    }, this.petitToHiddenDelay * 1000);
  }

  private scheduleHomeCollapse() {
    this.clear("homeCollapse");
    if (this.pinned) return;
    this.homeCollapse = window.setTimeout(() => {
      this.homeCollapse = null;
      if (this.state === "home") this.transition("petit");
    }, this.homeToPetitDelay * 1000);
  }

  private scheduleGreetCollapse(delay: number) {
    this.clear("greetCollapse");
    this.greetCollapse = window.setTimeout(() => {
      this.greetCollapse = null;
      if (this.state === "hello") this.transition("petit");
    }, delay * 1000);
  }

  private clear(which: "petitHide" | "homeCollapse" | "greetCollapse") {
    const id = this[which];
    if (id != null) window.clearTimeout(id);
    this[which] = null;
  }

  cancelTimers() {
    this.clear("petitHide");
    this.clear("homeCollapse");
    this.clear("greetCollapse");
  }

  private transition(next: FsmState) {
    if (this.petMode && next !== "hidden") return;
    if (next === this.state) return;
    const from = this.state;
    this.state = next;
    this.onTransition?.(from, next);
  }
}
