// The "now playing" strip: cover, title, artist, previous / play-pause / next, and the playing app's own speaker
// (mute) and volume slider (Windows' per-application audio session; never the master volume). Everything the
// player reports is set through textContent (h() never parses markup); the cover is an <img> whose source
// parseNowPlaying() already limited to a small base64 image data URL.

import { h, svg, clear } from "./dom";
import { ICONS } from "./icons";
import { Bridge } from "../core/bridge";
import { State } from "../core/state";
import {
  isSilent, isVisible, levelFromSlider, parseVolume, sliderFromLevel, speakerClick, subLine,
  type MediaAction, type MediaVolume, type NowPlaying,
} from "../integrations/media";

/** Extra island height while the strip is shown (strip + gap); mirrored in style.css (.overview.with-media). */
export const MEDIA_STRIP_H = 50; // strip 34 + 8 above + 8 of air above the island's bottom edge

export interface MediaStrip {
  el: HTMLElement;
  readonly visible: boolean;
  /** Redraws only when something it shows has changed. Returns true when it appeared or disappeared. */
  sync(): boolean;
  /** Reads the app's volume now (on demand: when the strip comes on screen and after each change; there is no polling). */
  refreshVolume(): void;
}

/** Slider drags are sent at most this often (plus the final value). */
const SEND_EVERY_MS = 90;

function button(title: string, icon: string, enabled: boolean, action: MediaAction, big = false): HTMLButtonElement {
  const b = h("button", { class: big ? "media-btn big" : "media-btn", title, "aria-label": title }, svg(icon, big ? 14 : 12));
  b.disabled = !enabled;
  // One explicit click = one command; nothing is retried or repeated.
  b.addEventListener("click", () => void Bridge.mediaControl(action));
  return b;
}

function draw(el: HTMLElement, m: NowPlaying, volume: HTMLElement) {
  clear(el);
  const art = m.thumbnail
    ? h("img", { class: "media-art", src: m.thumbnail, alt: "", draggable: false })
    : h("span", { class: "media-art none" }, svg(ICONS.musicNote, 14));
  el.append(
    art,
    h("div", { class: "media-text" },
      h("b", { class: "media-title", text: m.title || m.app, title: m.title }),
      h("span", { class: "media-sub", text: subLine(m), title: subLine(m) })),
    h("div", { class: "media-btns" },
      button("Anterior", ICONS.skipPrev, m.canPrevious, "previous"),
      button(m.status === "playing" ? "Pausa" : "Reproducir", m.status === "playing" ? ICONS.pause : ICONS.play, m.canPlayPause, "play_pause", true),
      button("Siguiente", ICONS.skipNext, m.canNext, "next")),
    volume,
  );
}

/** The speaker button and the slider. Both stay disabled while the app has no audio session. */
function buildVolume() {
  const speaker = h("button", { class: "media-btn", type: "button" });
  const slider = h("input", { class: "media-vol", type: "range", min: "0", max: "100", step: "1", "aria-label": "Volumen" });
  const el = h("div", { class: "media-vol-box" }, speaker, slider);
  let vol: MediaVolume | null = null;
  /** What Windows last said: the slider's optimistic state must not hide that the app is still muted there. */
  let serverMuted = false;
  let token = 0;
  let dragging = false;
  let timer: number | null = null;
  let pending: number | null = null;

  const paint = () => {
    const silent = isSilent(vol);
    const title = vol == null ? "Sin sesión de audio" : silent ? "Activar sonido" : "Silenciar";
    speaker.disabled = vol == null;
    slider.disabled = vol == null;
    speaker.title = title;
    speaker.setAttribute("aria-label", title);
    speaker.setAttribute("aria-pressed", String(silent));
    clear(speaker);
    speaker.append(svg(silent ? ICONS.speakerOff : ICONS.speakerOn, 13));
    if (!dragging) slider.value = String(vol ? (vol.muted ? 0 : sliderFromLevel(vol.level)) : 0);
    slider.style.setProperty("--fill", `${slider.value}%`);
    slider.title = vol == null ? "Sin sesión de audio" : `Volumen ${slider.value}%`;
  };
  const apply = (raw: unknown, mine: number) => {
    if (mine !== token) return; // a newer read or write has superseded this answer
    vol = parseVolume(raw);
    serverMuted = vol?.muted ?? false;
    paint();
  };
  const refresh = () => {
    const mine = ++token;
    void Bridge.mediaGetVolume().then((v) => apply(v, mine));
  };
  /** Runs a change, then shows what Windows says the state is now. */
  const change = async (run: () => Promise<unknown>[]) => {
    const mine = ++token;
    await Promise.all(run());
    if (mine === token) apply(await Bridge.mediaGetVolume(), mine);
  };

  speaker.addEventListener("click", () => {
    if (!vol) return;
    const want = speakerClick(vol);
    // Optimistic: the icon flips at once; the answer corrects it.
    vol = { muted: want.muted, level: want.level ?? vol.level };
    serverMuted = want.muted;
    paint();
    void change(() => [
      ...(want.level != null ? [Bridge.mediaSetVolume(want.level)] : []),
      Bridge.mediaSetMute(want.muted),
    ]);
  });

  const send = () => {
    timer = null;
    if (pending == null) return;
    const level = pending;
    pending = null;
    // Moving the slider up un-mutes, like every mixer does.
    void change(() => [Bridge.mediaSetVolume(level), ...(serverMuted && level > 0 ? [Bridge.mediaSetMute(false)] : [])]);
  };
  slider.addEventListener("pointerdown", () => { dragging = true; });
  const stop = () => { dragging = false; };
  slider.addEventListener("pointerup", stop);
  slider.addEventListener("pointercancel", stop);
  slider.addEventListener("blur", stop);
  slider.addEventListener("input", () => {
    if (!vol) return;
    const level = levelFromSlider(slider.value);
    vol = { muted: vol.muted && level <= 0, level };
    paint();
    pending = level;
    if (timer == null) timer = window.setTimeout(send, SEND_EVERY_MS);
  });
  slider.addEventListener("change", () => { dragging = false; if (timer != null) { clearTimeout(timer); send(); } });

  paint();
  return { el, refresh, reset() { vol = null; token++; paint(); } };
}

export function buildMediaStrip(opts: { stacked?: boolean } = {}): MediaStrip {
  const el = h("div", { class: opts.stacked ? "media-strip stacked" : "media-strip" });
  el.hidden = true;
  const volume = buildVolume();
  let key = "";
  let appKey = "";
  return {
    el,
    get visible() { return !el.hidden; },
    refreshVolume() { if (!el.hidden) volume.refresh(); },
    sync() {
      const m = State.media;
      const show = State.settings.mediaControl !== false && isVisible(m);
      if (!show) {
        if (el.hidden) return false;
        el.hidden = true; clear(el); key = ""; appKey = "";
        volume.reset();
        return true;
      }
      // Position is not part of the key: the card does not tick, so a seek must not redraw it.
      const next = [m.app, m.title, m.artist, m.status, m.thumbnail ?? "", m.canPlayPause, m.canNext, m.canPrevious].join("");
      if (next === key && !el.hidden) return false;
      key = next;
      const appeared = el.hidden;
      el.hidden = false;
      draw(el, m, volume.el);
      // A new app, a new track or a play/pause may have changed which audio session is live: read it again.
      const nextApp = `${m.app}${m.title}${m.status}`;
      if (appeared || nextApp !== appKey) { appKey = nextApp; volume.refresh(); }
      return appeared;
    },
  };
}
