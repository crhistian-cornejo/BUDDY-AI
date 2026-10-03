const HOST = 'io.github.crhistian_cornejo.buddy_youtube';
let port = null, connecting = false, pending = false, lastSent = 0;
let pendingAction = null;
// Transcript parts waiting to go to Buddy: one with each message, so every message fits the native host's frame.
const outbox = [];
const bytes = value => new TextEncoder().encode(JSON.stringify(value)).length;
function queueTranscript(videoId, result) {
  const lines = Array.isArray(result?.lines) ? result.lines : [];
  if (!lines.length) outbox.push({ videoId, error: String(result?.error || 'Este video no ofrece transcripción.').slice(0, 200) });
  else {
    const parts = [[]]; let size = 0;
    for (const line of lines) {
      const cost = bytes(line) + 1;
      if (parts.at(-1).length && size + cost > 30000) { parts.push([]); size = 0; }
      parts.at(-1).push(line); size += cost;
    }
    parts.forEach((part, i) => outbox.push({ videoId, part: i + 1, parts: parts.length, lang: String(result.lang || ''), lines: part }));
  }
  void send(true);
}
let connection = 'Abre Buddy y configura Video en Ajustes → Conexiones.';
const snapshots = new Map();
const browser = /Edg\//.test(navigator.userAgent) ? 'Edge' : 'Chrome';
function connect() {
  if (port || connecting) return;
  connecting = true;
  try {
    port = chrome.runtime.connectNative(HOST);
    port.onDisconnect.addListener(() => {
      const reason = chrome.runtime.lastError?.message;
      port = null; pending = false; connecting = false;
      connection = reason ? 'Configura la extensión desde los Ajustes de Buddy.' : 'Buddy está cerrado.';
    });
    port.onMessage.addListener(message => {
      pending = false;
      connection = message.enabled ? 'Conectado con Buddy ✓' : 'Activa Video en los Ajustes de Buddy.';
      if (pendingAction || outbox.length) void send(true);
      for (const command of message.commands || []) {
        if (['pause', 'toggle'].includes(command.type)) chrome.tabs.sendMessage(command.tabId, { type: command.type, videoId: command.videoId }).catch(() => {});
        if (command.type === 'transcript') chrome.tabs.sendMessage(command.tabId, { type: 'transcript', videoId: command.videoId })
          .then(result => queueTranscript(command.videoId, result), () => queueTranscript(command.videoId, { error: 'No se pudo leer la página del video. Recarga esa pestaña.' }));
        if (command.type === 'pip') void chrome.tabs.update(command.tabId, { active: true }).then(async tab => {
          await chrome.windows.update(tab.windowId, { focused: true });
          await chrome.action.openPopup({ windowId: tab.windowId });
        }).catch(() => { connection = 'Pulsa el icono de Buddy y elige Ver flotante.'; });
      }
    });
    connecting = false;
  } catch (_) { connecting = false; port = null; }
}
async function send(force = false) {
  if (pending || (!force && Date.now() - lastSent < 950)) return;
  connect(); if (!port) return;
  pending = true;
  try {
    const focused = await chrome.windows.getLastFocused().catch(() => null);
    const tabs = await chrome.tabs.query({});
    const live = new Map(tabs.map(t => [t.id, t]));
    for (const [key, state] of snapshots) if (!live.has(state.tabId) || Date.now() - state.at > 15000) snapshots.delete(key);
    const videos = [...snapshots.values()].flatMap(state => {
      const tab = live.get(state.tabId); if (!tab) return [];
      const v = state.snapshot;
      return [{ videoId: v.videoId, service: v.service || 'youtube', url: v.url, seconds: v.seconds, duration: v.duration,
        caption: v.caption, playing: v.playing, tabId: tab.id, title: (state.title || tab.title || 'Video').replace(/ - YouTube$/, '').slice(0, 240),
        active: Boolean(tab.active && tab.windowId === focused?.id && v.visible) }];
    }).sort((a, b) => Number(b.active) - Number(a.active) || Number(b.playing) - Number(a.playing)).slice(0, 20);
    const transcript = outbox.shift();
    while (videos.length && bytes({ browser, videos, transcript }) > 60000) videos.pop();
    const action = pendingAction; pendingAction = null;
    lastSent = Date.now(); port.postMessage(transcript ? { browser, videos, action, transcript } : { browser, videos, action });
  } catch (_) { pending = false; port = null; }
}
chrome.runtime.onMessage.addListener((message, sender, reply) => {
  if (message.type === 'open-companion' && !sender.tab && ['notch', 'floating'].includes(message.destination)) {
    void chrome.tabs.query({ active: true, currentWindow: true }).then(tabs => {
      const found = [...snapshots.values()].find(s => s.tabId === tabs[0]?.id && (!s.snapshot.service || s.snapshot.service === 'youtube'));
      if (!found || !port || !connection.includes('Conectado')) { reply({ ok: false }); return; }
      pendingAction = { type: 'open', videoId: found.snapshot.videoId, destination: message.destination };
      void send(true); reply({ ok: true });
    }); return true;
  }
  if (message.type === 'status' && !sender.tab) { reply({ connection }); void send(true); return; }
  if (message.type !== 'snapshot' || !sender.tab) return;
  let origin; try { origin = new URL(sender.url).origin; } catch (_) { return; }
  const accept = () => {
    const key = `${sender.tab.id}:${sender.frameId || 0}`;
    if (message.snapshot) snapshots.set(key, { tabId: sender.tab.id, origin, title: sender.tab.title, snapshot: message.snapshot, at: Date.now() });
    else snapshots.delete(key);
    void send(true);
  };
  if (origin === 'https://www.youtube.com') accept();
  else if (origin.startsWith('https://')) void chrome.permissions.contains({ origins: [`${origin}/*`] }).then(ok => { if (ok) accept(); });
});
chrome.tabs.onRemoved.addListener(id => { for (const [key, state] of snapshots) if (state.tabId === id) snapshots.delete(key); void send(true); });
chrome.tabs.onActivated.addListener(() => void refresh());
chrome.tabs.onUpdated.addListener((_id, change) => { if (change.url || change.status === 'complete') void refresh(); });
async function refresh() {
  const tabs = await chrome.tabs.query({}).catch(() => []);
  for (const tab of tabs) chrome.tabs.sendMessage(tab.id, { type: 'refresh' }).catch(() => {});
  void send(true);
}
// One lightweight heartbeat; video positions come from playback events, not a per-second idle loop.
setInterval(() => { void refresh(); }, 10000);
void refresh();

chrome.permissions.onRemoved?.addListener(() => {
  void Promise.all([...snapshots.entries()].map(async ([key, value]) => {
    if (value.origin !== 'https://www.youtube.com' && !await chrome.permissions.contains({ origins: [value.origin + '/*'] })) snapshots.delete(key);
  })).then(() => send(true));
});
