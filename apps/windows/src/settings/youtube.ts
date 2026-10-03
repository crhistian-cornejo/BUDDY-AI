import { invoke } from '@tauri-apps/api/core';
import { h } from '../chat/dom';
import { button, section } from './ui';
import type { YouTubeStatus } from '../bar/youtube';

export function renderYouTube(): { element: HTMLElement; refresh: () => Promise<void> } {
  const { el, card } = section('YouTube');
  const status = h('p', { role: 'status', class: 'row-title' });
  const message = h('p', { role: 'status', class: 'muted small' });
  const browser = h('select', { class: 'select', 'aria-label': 'Navegador' }, h('option', { value: 'chrome' }, 'Chrome'), h('option', { value: 'edge' }, 'Edge')) as HTMLSelectElement;
  const steps = h('div', { hidden: true },
    h('p', { class: 'small', style: 'white-space:pre-line', text: '1. Abre Extensiones y activa Modo desarrollador.\n2. Pulsa Cargar descomprimida y elige la carpeta preparada.\n3. Recarga tu pestaña de YouTube.' }),
    button('Abrir extensiones', () => void run(() => invoke('youtube_browser_page', { browser: browser.value }))),
    button('Mostrar carpeta', () => void run(() => invoke('youtube_show_folder'))),
    button('Copiar ubicación', () => void run(() => invoke('youtube_copy_folder'))));
  async function run(work: () => Promise<unknown>) { try { await work(); message.textContent = ''; await refresh(); } catch (error) { message.textContent = String(error); } }
  const off = button('Desactivar', () => void run(() => invoke('youtube_enable', { enabled: false })));
  card.append(status, browser,
    h('div', { class: 'row-btns' }, button('Configurar extensión…', () => void run(async () => {
      await invoke('youtube_prepare', { browser: browser.value });
      steps.hidden = false;
      await invoke('youtube_browser_page', { browser: browser.value });
    })), button('Comprobar', () => void refresh()), off), steps, message);
  el.append(h('p', { class: 'muted small', text: 'Gratis, sin claves API ni cuentas adicionales. YouTube está habilitado; puedes activar otros sitios desde el icono de la extensión. El navegador requiere que tú la habilites una vez.' }));
  async function refresh() {
    const state = await invoke<YouTubeStatus>('youtube_status');
    status.textContent = state.connected ? 'Conectado ✓' : state.enabled ? 'Esperando la extensión' : 'Sin configurar';
    off.hidden = !state.enabled;
    steps.hidden = !(await invoke<string>('youtube_extension_path'));
  }
  void refresh();
  return { element: el, refresh };
}
