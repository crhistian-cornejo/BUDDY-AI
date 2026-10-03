import { invoke } from '@tauri-apps/api/core';
import { h, svg } from '../chat/dom';

export interface YouTubeVideo { sourceId: string; tabId: number; videoId: string; title: string; browser: string; seconds: number; duration: number | null; playing: boolean; caption: string; service: string; url: string }
export interface YouTubeStatus { enabled: boolean; connected: boolean; detected: YouTubeVideo | null; viewer: YouTubeVideo | null; destination: string }

export class YouTubePanel {
  state: YouTubeStatus = { enabled: false, connected: false, detected: null, viewer: null, destination: "notch" };
  private frame: HTMLIFrameElement | null = null;
  private key = '';
  private error = '';
  private status = h('span', { class: 'muted', role: 'status' });
  constructor(private change: (started: boolean) => void) {
    window.addEventListener('message', event => {
      if (!this.frame || event.source !== this.frame.contentWindow || event.origin !== location.origin) return;
      const v = this.state.viewer, data = event.data;
      if (!v || data?.channel !== 'buddy-youtube' || data.videoId !== v.videoId) return;
      if (data.type === 'position') void invoke('youtube_position', { sourceId: v.sourceId, videoId: v.videoId, seconds: data.code });
      if (data.type === 'playing') void invoke('youtube_started', { sourceId: v.sourceId, videoId: v.videoId });
      if (data.type === 'error') this.error = [101, 150].includes(data.code) ? 'Este video solo se puede ver en YouTube.' : 'No se pudo reproducir. Puedes volver a YouTube.';
      if (data.type === 'blocked') this.error = 'Pulsa reproducir en el video para continuar.';
      this.status.textContent = this.error || v.title;
    });
  }
  async refresh() {
    const before = this.state.destination === "notch" && !!this.state.viewer;
    this.state = await invoke<YouTubeStatus>('youtube_status');
    this.change(!before && this.state.destination === "notch" && !!this.state.viewer);
  }
  close() {
    this.frame?.remove(); this.frame = null; this.key = '';
    this.state.viewer = null;
    void invoke('youtube_close');
  }
  draw(open: boolean) {
    const prompt = document.getElementById('youtube-prompt')!;
    const pane = document.getElementById('youtube-viewer')!;
    const v = this.state.destination === "notch" ? this.state.viewer : null;
    prompt.hidden = !open || !!v || !this.state.detected;
    if (!prompt.hidden && this.state.detected) {
      const detected = this.state.detected;
      const icon = (path: string) => svg(path, 20, { fill: 'none', stroke: 'currentColor', 'stroke-width': '1.75', 'stroke-linecap': 'round', 'stroke-linejoin': 'round' });
      const clock = (seconds: number) => `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, '0')}`;
      const open = () => {
        void invoke(detected.service === 'youtube' ? 'youtube_open' : 'video_browser_pip', { sourceId: detected.sourceId, videoId: detected.videoId }).then(() => {
          document.getElementById('tool-message')!.hidden = true;
          return this.refresh();
        }).catch(() => {
          document.getElementById('tool-message')!.textContent = 'El video cambió o el navegador se desconectó. Vuelve a intentarlo.';
          document.getElementById('tool-message')!.hidden = false;
        });
      };
      const progress = h('div', { class: 'progress' }, h('span', { text: clock(detected.seconds) }));
      if (detected.duration) progress.append(h('div', { class: 'bar-track' }, h('div', { class: 'bar-fill', style: `width:${Math.min(100, detected.seconds / detected.duration * 100)}%` })),
        h('span', { text: `-${clock(Math.max(0, detected.duration - detected.seconds))}` }));
      else progress.append(h('span', { text: detected.playing ? 'Reproduciendo' : 'En pausa' }));
      prompt.className = 'tile player';
      prompt.replaceChildren(detected.service === 'youtube' ? h('img', { src: `https://i.ytimg.com/vi/${detected.videoId}/hqdefault.jpg`, alt: '', referrerpolicy: 'no-referrer' }) : h('span', { text: '▶', style: 'width:64px;text-align:center;font-size:28px' }),
        h('div', { class: 'track' }, h('strong', { text: detected.title, title: detected.title }), h('span', { class: 'muted', text: `${detected.service === "youtube" ? "YouTube" : "Video"} · ${detected.browser}` }), progress),
        h('div', { class: 'controls' },
          h('button', { class: 'icon-btn', type: 'button', title: detected.playing ? 'Pausar en el navegador' : 'Reproducir en el navegador', 'aria-label': detected.playing ? 'Pausar YouTube' : 'Reproducir YouTube',
            onclick: () => void invoke('youtube_toggle', { sourceId: detected.sourceId, videoId: detected.videoId }) },
            icon(detected.playing ? 'M6 4h4v16H6z M14 4h4v16h-4z' : 'M6 4l14 8l-14 8z')),
          h('button', { class: 'icon-btn', type: 'button', title: detected.service === 'youtube' ? 'Reproducir en el notch' : 'Abrir extensión para ver flotante', 'aria-label': 'Ver video', onclick: open },
            icon('M3 3h18v18H3z M12 12h7v7h-7z M5 5l5 5 M5 10h5V5')),
          ...(detected.service === 'youtube' ? [h('button', { class: 'icon-btn', type: 'button', title: 'Ver junto a Buddy', 'aria-label': 'Ver junto a Buddy', onclick: () => void invoke('youtube_open_floating', { sourceId: detected.sourceId, videoId: detected.videoId }) }, icon('M3 5h18v14H3z M3 9h18'))] : []),
          h('button', { class: 'icon-btn', type: 'button', text: '✧', title: 'Preguntar a Gemini', onclick: () => void invoke('video_ask') })));

    }
    pane.hidden = !open || !v;
    if (!open || !v) {
      this.frame?.remove(); this.frame = null; this.key = '';
      pane.replaceChildren();
      if (v) this.close();
      return;
    }
    const key = v.sourceId + v.videoId;
    if (key === this.key) return; // detection events never reload the selected player
    this.key = key; this.error = '';
    this.frame = h('iframe', { src: `youtube.html#${v.videoId}/${Math.floor(v.seconds)}`, title: `YouTube · ${v.title}`, allow: 'autoplay; encrypted-media; fullscreen; picture-in-picture', referrerpolicy: 'strict-origin-when-cross-origin' }) as HTMLIFrameElement;
    this.status.textContent = v.title;
    pane.replaceChildren(this.frame, h('div', { class: 'youtube-actions' }, this.status,
      h('button', { class: 'pill', type: 'button', text: 'Abrir YouTube', onclick: () => void invoke('open_url', { url: `https://www.youtube.com/watch?v=${v.videoId}` }) }),
      h('button', { class: 'pill', type: 'button', text: 'Junto a Buddy', onclick: () => void invoke('youtube_move', { destination: 'floating' }) }),
      h('button', { class: 'pill', type: 'button', text: 'Gemini', onclick: () => void invoke('video_ask') }),
      h('button', { class: 'pill', type: 'button', text: 'Cerrar video', onclick: () => { this.close(); this.change(false); } })));
  }
}
