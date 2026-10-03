const { test } = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const path = require('node:path');

function content(href) {
  const reports = [], listeners = {};
  const video = { currentTime: 42.5, duration: 120, paused: false, ended: false,
    addEventListener() {}, removeEventListener() {}, pause() { this.paused = true; }, async play() { this.paused = false; } };
  const sandbox = { URL, location: { href }, Date, Number, MutationObserver: class { observe() {} },
    document: { visibilityState: 'visible', documentElement: {}, addEventListener() {}, querySelector: q => q === 'video' ? video : null },
    window: { addEventListener() {} }, chrome: { runtime: { id: 'abc', sendMessage: async m => reports.push(m), onMessage: { addListener: f => { listeners.message = f; } } } } };
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'content.js'), 'utf8'), sandbox);
  return { video, reports, listeners };
}

test('detects watch, Shorts and live without links and pauses only the matching video', () => {
  for (const url of ['https://www.youtube.com/watch?v=abcdefghijk', 'https://www.youtube.com/shorts/abcdefghijk', 'https://www.youtube.com/live/abcdefghijk']) {
    const c = content(url);
    assert.equal(c.reports[0].snapshot.videoId, 'abcdefghijk');
    assert.equal(c.reports[0].snapshot.seconds, 42.5);
    assert.equal(c.reports[0].snapshot.duration, 120);
    c.listeners.message({ type: 'pause', videoId: 'lmnopqrstuv' }, {}, () => {});
    assert.equal(c.video.paused, false);
    c.listeners.message({ type: 'pause', videoId: 'abcdefghijk' }, {}, () => {});
    assert.equal(c.video.paused, true);
    c.listeners.message({ type: 'toggle', videoId: 'lmnopqrstuv' }, {}, () => {});
    assert.equal(c.video.paused, true);
    c.listeners.message({ type: 'toggle', videoId: 'abcdefghijk' }, {}, () => {});
    assert.equal(c.video.paused, false);
  }
  assert.equal(content('https://www.youtube.com/').reports[0].snapshot, null);
});

test('background prioritizes the focused window, caps messages and relays a tab-specific pause', async () => {
  const posted = [], commands = [], listeners = {};
  const tabs = Array.from({ length: 25 }, (_, i) => ({ id: i, windowId: i === 24 ? 2 : 1, active: true, title: 'x'.repeat(2000) }));
  const port = { postMessage: m => posted.push(m), onDisconnect: { addListener() {} }, onMessage: { addListener: f => { listeners.native = f; } } };
  const sandbox = { navigator: { userAgent: 'Edg/130' }, TextEncoder, URL, Date, Map, Set, Boolean, Number, setInterval() {}, chrome: {
    permissions: {}, runtime: { connectNative: () => port, onMessage: { addListener: f => { listeners.message = f; } } },
    windows: { getLastFocused: async () => ({ id: 2 }) },
    tabs: { query: async () => tabs, sendMessage: async (id, m) => { commands.push({ id, ...m }); }, onRemoved: { addListener() {} }, onActivated: { addListener() {} }, onUpdated: { addListener() {} } } } };
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'background.js'), 'utf8'), sandbox);
  await new Promise(setImmediate);
  listeners.native({ enabled: true, commands: [] });
  for (const tab of tabs) listeners.message({ type: 'snapshot', snapshot: { videoId: 'abcdefghijk', seconds: 10, playing: true, visible: true } }, { tab, url: 'https://www.youtube.com/watch?v=abcdefghijk' }, () => {});
  await new Promise(setImmediate);
  const data = posted.at(-1);
  assert.equal(data.browser, 'Edge');
  assert.equal(data.videos.length, 20);
  assert.equal(data.videos[0].tabId, 24);
  assert.equal(data.videos[0].active, true);
  assert.equal(data.videos[1].active, false);
  assert.equal(data.videos[0].title.length, 240);
  listeners.native({ enabled: true, commands: [{ type: 'pause', tabId: 24, videoId: 'abcdefghijk' }] });
  await new Promise(setImmediate);
  assert.ok(commands.some(c => c.id === 24 && c.type === 'pause' && c.videoId === 'abcdefghijk'));
  listeners.native({ enabled: true, commands: [] });
  let opened;
  listeners.message({ type: 'open-companion', destination: 'floating' }, {}, reply => { opened = reply; });
  await new Promise(setImmediate);
  assert.equal(opened.ok, true);
  assert.equal(posted.at(-1).action.destination, 'floating');
  assert.equal(posted.at(-1).action.videoId, 'abcdefghijk');
});

function popup(video) {
  const elements = new Map();
  const el = id => {
    if (!elements.has(id)) elements.set(id, { textContent: '', disabled: false, hidden: false, classList: { toggle() {} }, addEventListener(type, cb) { this[type] = cb; } });
    return elements.get(id);
  };
  const calls = [];
  const sandbox = { URL, document: { getElementById: el, querySelectorAll: () => video ? [video] : [], pictureInPictureEnabled: true },
    chrome: { runtime: { getManifest: () => ({ version: '0.3.0' }), sendMessage: async () => ({ connection: 'Conectado' }) },
      tabs: { query: async () => [{ id: 7, url: 'https://video.example/watch/demo?token=private' }] },
      permissions: { contains: async () => false, request: async p => { calls.push(p); return true; } },
      scripting: { unregisterContentScripts: async () => {}, registerContentScripts: async p => calls.push(p), executeScript: async p => p.func ? [{ result: await p.func() }] : [] } } };
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'popup.js'), 'utf8'), sandbox);
  return { el, calls };
}
test('browser PiP respects platform restrictions and optional permissions require an explicit click', async () => {
  let requested = 0;
  const v = { paused: false, readyState: 4, clientWidth: 800, clientHeight: 450, disablePictureInPicture: true, async requestPictureInPicture() { requested++; } };
  const p = popup(v); await new Promise(setImmediate);
  assert.equal(p.calls.length, 0);
  await p.el('pip').click(); assert.equal(requested, 0); assert.match(p.el('feedback').textContent, /no permite/);
  v.disablePictureInPicture = false; await p.el('pip').click(); assert.equal(requested, 1);
  await p.el('enable').click(); assert.equal(p.calls[0].origins[0], 'https://video.example/*');
  assert.equal(p.calls[1][0].matches[0], 'https://video.example/*');
});

// The transcript: read from the panel YouTube itself shows, opened and closed again if it was not open.
function transcriptPage({ offered = true, appears = true } = {}) {
  const clicks = [];
  let open = false;
  const segment = (time, text) => ({ innerText: `${time}\n${text}` });
  const segments = [segment('0:00', 'Hola a todos'), segment('4:35', 'aquí entra   la función\nde activación'), segment('1:02:05', 'gracias por ver'), { innerText: 'Capítulo 2' }];
  const openButton = { click() { clicks.push('open'); if (appears) open = true; }, getAttribute: () => 'Mostrar transcripción' };
  const closeButton = { click() { clicks.push('close'); open = false; } };
  const video = { currentTime: 281, duration: 3144, paused: false, ended: false, addEventListener() {}, removeEventListener() {} };
  const listeners = {};
  const sandbox = { URL, location: { href: 'https://www.youtube.com/watch?v=abcdefghijk' }, Date, Number, Promise, setTimeout: f => setImmediate(f),
    MutationObserver: class { observe() {} }, window: { addEventListener() {} },
    document: { visibilityState: 'visible', documentElement: {}, addEventListener() {},
      querySelector: q => q === 'video' ? video
        : q.startsWith('ytd-video-description-transcript-section-renderer') ? (offered ? openButton : null)
        : q.includes('#visibility-button') || q.includes('transcript') ? (open ? closeButton : null) : null,
      querySelectorAll: q => q.includes('transcript-segment') ? (open ? segments : []) : [] },
    chrome: { runtime: { id: 'abc', sendMessage: async () => {}, onMessage: { addListener: f => { listeners.message = f; } } } } };
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'content.js'), 'utf8'), sandbox);
  const ask = videoId => new Promise(resolve => { if (listeners.message({ type: 'transcript', videoId }, {}, resolve) !== true) resolve(undefined); });
  return { ask, clicks };
}

test('reads the transcript the page shows, with its times, and leaves the page as it was', async () => {
  const page = transcriptPage();
  const result = await page.ask('abcdefghijk');
  assert.deepEqual(JSON.parse(JSON.stringify(result.lines)), [[0, 'Hola a todos'], [275, 'aquí entra la función de activación'], [3725, 'gracias por ver']]);
  assert.deepEqual(page.clicks, ['open', 'close']);
  // Another video's transcript is never read from this tab.
  assert.equal(await transcriptPage().ask('lmnopqrstuv'), undefined);
});

test('says so when the video offers no transcript or YouTube does not show it', async () => {
  assert.match((await transcriptPage({ offered: false }).ask('abcdefghijk')).error, /no ofrece transcripción/);
  const silent = transcriptPage({ appears: false });
  assert.match((await silent.ask('abcdefghijk')).error, /no mostró/);
});

test('background asks the tab for the transcript and hands it to Buddy in parts that fit a frame', async () => {
  const posted = [], listeners = {};
  const lines = Array.from({ length: 900 }, (_, i) => [i * 4, `línea número ${i} con algo de texto para ocupar espacio — ñandú`]);
  const tab = { id: 7, windowId: 1, active: true, title: 'Demo' };
  const port = { postMessage: m => posted.push(m), onDisconnect: { addListener() {} }, onMessage: { addListener: f => { listeners.native = f; } } };
  const sandbox = { navigator: { userAgent: 'Chrome/130' }, TextEncoder, URL, Date, Map, Set, Boolean, Number, String, Array, setInterval() {}, chrome: {
    permissions: {}, runtime: { connectNative: () => port, onMessage: { addListener: f => { listeners.message = f; } } },
    windows: { getLastFocused: async () => ({ id: 1 }) },
    tabs: { query: async () => [tab], sendMessage: async (id, m) => m.type === 'transcript' && id === 7 ? { lines } : {}, onRemoved: { addListener() {} }, onActivated: { addListener() {} }, onUpdated: { addListener() {} } } } };
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'background.js'), 'utf8'), sandbox);
  const settle = async () => { for (let i = 0; i < 20; i++) await new Promise(setImmediate); };
  await settle();
  listeners.native({ enabled: true, commands: [{ type: 'transcript', tabId: 7, videoId: 'abcdefghijk' }] });
  // Each reply from Buddy lets the next part leave.
  for (let i = 0; i < 12; i++) { await settle(); listeners.native({ enabled: true, commands: [] }); }
  await settle();
  const parts = posted.filter(m => m.transcript).map(m => m.transcript);
  assert.ok(parts.length >= 2, `in several parts: ${parts.length}`);
  assert.deepEqual(parts.map(p => p.part), parts.map((_, i) => i + 1));
  assert.ok(parts.every(p => p.parts === parts.length && p.videoId === 'abcdefghijk'));
  assert.equal(parts.reduce((n, p) => n + p.lines.length, 0), 900);
  for (const message of posted) assert.ok(new TextEncoder().encode(JSON.stringify(message)).length < 64 * 1024, 'fits the native host frame');
  // A tab that cannot be read answers with the reason, not with silence.
  sandbox.chrome.tabs.sendMessage = async () => { throw new Error('no receiver'); };
  listeners.native({ enabled: true, commands: [{ type: 'transcript', tabId: 9, videoId: 'lmnopqrstuv' }] });
  for (let i = 0; i < 3; i++) { await settle(); listeners.native({ enabled: true, commands: [] }); }
  assert.ok(posted.some(m => m.transcript?.videoId === 'lmnopqrstuv' && /Recarga/.test(m.transcript.error)));
});

// After the extension is reloaded, the copy of the script left in an open tab can no longer talk to it.
test('a script orphaned by an extension reload stops quietly instead of throwing', () => {
  const handlers = {}; let observing = true, removed = 0;
  const video = { currentTime: 1, duration: 10, paused: false, ended: false, addEventListener: (name, f) => { handlers[name] = f; }, removeEventListener() { removed++; } };
  const runtime = { id: 'abc', sendMessage: async () => {}, onMessage: { addListener() {} } };
  const sandbox = { URL, location: { href: 'https://www.youtube.com/watch?v=abcdefghijk' }, Date, Number, Promise, Boolean, setTimeout,
    MutationObserver: class { observe() {} disconnect() { observing = false; } }, window: { addEventListener() {} },
    document: { visibilityState: 'visible', documentElement: {}, addEventListener: (name, f) => { handlers[name] = f; }, querySelector: q => q === 'video' ? video : null, querySelectorAll: () => [] },
    chrome: { runtime } };
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'content.js'), 'utf8'), sandbox);
  // The reload: the runtime loses its id and every call into it throws.
  delete runtime.id;
  runtime.sendMessage = () => { throw new Error('Extension context invalidated.'); };
  assert.doesNotThrow(() => handlers.visibilitychange());
  assert.doesNotThrow(() => handlers.seeked());
  assert.equal(observing, false, 'it stops watching the page');
  assert.ok(removed > 0, 'and lets go of the video');
});
