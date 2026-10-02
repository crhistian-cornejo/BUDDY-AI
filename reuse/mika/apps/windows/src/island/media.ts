// Wires the media strip to Rust: keeps State.media current while the window is on screen (the island in its compact
// bar or open, the floating Mika while she is shown) and the setting is on, and tells Rust to stop listening the moment
// it is not. While the island is hidden (or paused, or the setting is off) Rust has no handlers registered and no
// thread running. The worker only wakes when Windows reports a change: no polling.

import { Bridge, onEvent } from "../core/bridge";
import { State } from "../core/state";
import { parseNowPlaying, wantWatch } from "../integrations/media";

/** `shown` says whether this window is on screen (the island: not hidden; Mika: visible). */
export async function registerMedia(shown: () => boolean = () => State.mode !== "hidden") {
  let watching = false;
  let token = 0;

  const set = (value: unknown) => {
    const next = parseNowPlaying(value);
    if (JSON.stringify(next) === JSON.stringify(State.media)) return;
    State.media = next;
    State.notify();
  };

  await onEvent<unknown>("media-changed", (payload) => {
    if (watching) set(payload);
  });

  const evaluate = () => {
    const want = wantWatch({
      enabled: State.settings.mediaControl !== false,
      shown: shown(),
      paused: State.paused,
    });
    if (want === watching) return;
    watching = want;
    const mine = ++token;
    void Bridge.mediaWatch(want);
    if (want) {
      // The worker also reports on start; this covers the event racing the listener.
      void Bridge.mediaNowPlaying().then((v) => { if (watching && mine === token) set(v); });
    } else if (State.media) {
      State.media = null;
      State.notify();
    }
  };

  State.subscribe(evaluate);
  evaluate();
}
