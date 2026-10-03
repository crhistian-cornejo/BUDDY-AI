// Runs only on a site the user explicitly enabled. No cookies, stream URLs or protected media are read.
(() => {
  if (location.hostname === 'www.youtube.com') return;
  if (window.__buddyVideo) return;
  window.__buddyVideo = true;
  let bound, last = 0;
  const videoId = () => {
    let value = 2166136261;
    for (const c of location.origin + location.pathname) value = Math.imul(value ^ c.charCodeAt(0), 16777619);
    return ('browser' + (value >>> 0).toString(36)).padEnd(11, '_').slice(0, 11);
  };
  const choose = () => [...document.querySelectorAll('video')].sort((a, b) =>
    Number(!b.paused) - Number(!a.paused) || b.clientWidth * b.clientHeight - a.clientWidth * a.clientHeight)[0];
  function report(force = false) {
    if (!force && Date.now() - last < 1000) return;
    last = Date.now();
    const video = bound, seconds = video?.currentTime;
    const url = location.origin + location.pathname;
    const caption = video ? [...video.textTracks].filter(t => t.mode === 'showing').flatMap(t => [...(t.activeCues || [])].map(c => c.text || '')).join('\n').slice(0, 2000) : '';
    const snapshot = video && Number.isFinite(seconds) && video.readyState > 0 ?
      { service: 'browser', videoId: videoId(), url, seconds, duration: Number.isFinite(video.duration) ? video.duration : null,
        caption, playing: !video.paused && !video.ended, visible: document.visibilityState === 'visible' } : null;
    chrome.runtime.sendMessage({ type: 'snapshot', snapshot }).catch(() => {});
  }
  function changed() { report(); }
  function bind() {
    const next = choose();
    if (bound !== next) {
      if (bound) for (const event of ['play', 'pause', 'ended', 'seeked', 'timeupdate', 'loadedmetadata']) bound.removeEventListener(event, changed);
      bound = next;
      if (bound) for (const event of ['play', 'pause', 'ended', 'seeked', 'timeupdate', 'loadedmetadata']) bound.addEventListener(event, changed);
    }
    report(true);
  }
  chrome.runtime.onMessage.addListener((m, _sender, reply) => {
    if (m.type === 'refresh') { bind(); reply({ ok: true }); }
    else if (['pause', 'toggle'].includes(m.type) && m.videoId === videoId() && bound) {
      if (m.type === 'toggle' && bound.paused) void bound.play().catch(() => {}); else bound.pause();
      report(true); reply({ ok: true });
    }
  });
  document.addEventListener('visibilitychange', () => report(true));
  window.addEventListener('pagehide', () => chrome.runtime.sendMessage({ type: 'snapshot', snapshot: null }).catch(() => {}));
  new MutationObserver(() => { if (choose() !== bound) bind(); }).observe(document.documentElement, { childList: true, subtree: true });
  bind();
})();
