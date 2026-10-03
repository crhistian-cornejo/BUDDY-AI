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
    window: { addEventListener() {} }, chrome: { runtime: { sendMessage: async m => reports.push(m), onMessage: { addListener: f => { listeners.message = f; } } } } };
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
