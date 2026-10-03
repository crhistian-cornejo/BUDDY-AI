import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { h } from '../chat/dom';
import { applyTokens } from '../tokens';
import type { YouTubeStatus } from './youtube';
import './video-window.css';
applyTokens();
const root = document.getElementById('video')!;
let frame: HTMLIFrameElement | null = null, key = '', state: YouTubeStatus;
let request = 0;
const note = h('p', { class: 'video-note' });
window.addEventListener('message', event => {
  const v = state?.viewer, data = event.data;
  if (!v || event.source !== frame?.contentWindow || event.origin !== location.origin || data?.channel !== 'buddy-youtube' || data.videoId !== v.videoId) return;
  if (data.type === 'position') void invoke('youtube_position', { sourceId: v.sourceId, videoId: v.videoId, seconds: data.code });
  if (data.type === 'playing') void invoke('youtube_started', { sourceId: v.sourceId, videoId: v.videoId });
  if (data.type === 'error') note.textContent = 'Este video no permite reproducción aquí. Ábrelo en YouTube.';
  if (data.type === 'blocked') note.textContent = 'Pulsa reproducir en el video.';
});
async function refresh() {
  const revision = ++request;
  const next = await invoke<YouTubeStatus>('youtube_status');
  if (revision !== request) return;
  state = next;
  const v = state.viewer;
  if (!v || state.destination !== 'floating') { frame?.remove(); frame = null; await getCurrentWindow().close(); return; }
  const nextKey = v.sourceId + v.videoId;
  if (key === nextKey) return;
  key = nextKey; frame?.remove();
  frame = h('iframe', { src: `youtube.html#${v.videoId}/${Math.floor(v.seconds)}`, title: v.title, allow: 'autoplay; encrypted-media; fullscreen; picture-in-picture', referrerpolicy: 'strict-origin-when-cross-origin' }) as HTMLIFrameElement;
  note.textContent = v.title;
  root.replaceChildren(frame, h('footer', {}, note, h('div', { class: 'video-actions' },
    h('button', { text: 'Preguntar a Gemini', title: 'Título, minuto y subtítulos disponibles', onclick: () => void invoke('video_ask') }),
    h('button', { text: 'Al notch', onclick: () => void invoke('youtube_move', { destination: 'notch' }) }),
    h('button', { text: 'Abrir YouTube', onclick: () => void invoke('open_url', { url: v.url }) }))));
}
void listen<{ type: string }>('core-event', ({ payload }) => { if (payload.type === 'youTubeChanged') void refresh(); }).then(refresh);
document.addEventListener('visibilitychange', () => { if (document.hidden) { frame?.remove(); frame = null; void invoke('youtube_close'); } });
