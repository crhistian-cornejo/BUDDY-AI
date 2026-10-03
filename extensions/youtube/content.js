// Local playback state, and the transcript YouTube itself shows on the page when Buddy is asked about the video.
// Never access a media URL, account, cookie or private YouTube API.
(() => {
  let last = 0, bound;
  const id = () => {
    const u = new URL(location.href);
    const v = u.pathname === '/watch' ? u.searchParams.get('v') : /^\/(shorts|live)\/([\w-]{11})/.exec(u.pathname)?.[2];
    return /^[\w-]{11}$/.test(v || '') ? v : null;
  };
  function report(force = false) {
    if (!force && Date.now() - last < 1000) return;
    last = Date.now();
    const video = document.querySelector('video');
    const videoId = id();
    const seconds = video?.currentTime;
    const snapshot = videoId && video && Number.isFinite(seconds) && !document.querySelector('.ad-showing')
      ? { videoId, seconds, duration: Number.isFinite(video.duration) ? video.duration : null, caption: (document.querySelector('.ytp-caption-window-container')?.innerText || '').slice(0, 2000), playing: !video.paused && !video.ended, visible: document.visibilityState === 'visible' } : null;
    chrome.runtime.sendMessage({ type: 'snapshot', snapshot }).catch(() => {});
  }
  function bind() {
    const video = document.querySelector('video');
    if (video !== bound) {
      if (bound) for (const event of ['play', 'pause', 'ended', 'seeked', 'timeupdate', 'loadedmetadata']) bound.removeEventListener(event, changed);
      bound = video;
      if (bound) for (const event of ['play', 'pause', 'ended', 'seeked', 'timeupdate', 'loadedmetadata']) bound.addEventListener(event, changed);
    }
    report(true);
  }
  function changed() { report(); }
  // The transcript, as the page's own panel lists it: «0:05» and what is said then.
  const SEGMENTS = 'ytd-transcript-segment-renderer, transcript-segment-view-model';
  function lines() {
    const found = [];
    for (const segment of document.querySelectorAll(SEGMENTS)) {
      const match = /^\s*((?:\d+:)?\d{1,2}:\d{2})\s*([\s\S]*)$/.exec(segment.innerText || segment.textContent || '');
      if (!match) continue;
      const text = match[2].replace(/\s+/g, ' ').trim().slice(0, 400);
      if (text) found.push([match[1].split(':').reduce((total, part) => total * 60 + Number(part), 0), text]);
    }
    return found;
  }
  // Opens the page's transcript panel when it is not open, reads it, and closes it again.
  async function transcript() {
    let found = lines();
    if (found.length) return { lines: found };
    const open = document.querySelector('ytd-video-description-transcript-section-renderer button')
      || [...document.querySelectorAll('#description button, ytd-watch-metadata button')].find(b => /transcri/i.test(b.getAttribute('aria-label') || b.innerText || ''));
    if (!open) return { error: 'Este video no ofrece transcripción.' };
    open.click();
    for (let tries = 0; tries < 40 && !found.length; tries++) {
      await new Promise(done => setTimeout(done, 200));
      found = lines();
    }
    (document.querySelector('ytd-engagement-panel-section-list-renderer[target-id*="transcript"] #visibility-button button')
      || document.querySelector('button[aria-label*="Cerrar transcri" i], button[aria-label*="Close transcript" i]'))?.click();
    return found.length ? { lines: found } : { error: 'YouTube no mostró la transcripción de este video.' };
  }
  chrome.runtime.onMessage.addListener((message, _sender, reply) => {
    if (['pause', 'toggle'].includes(message.type) && message.videoId === id()) {
      const video = document.querySelector('video');
      if (video) {
        if (message.type === 'toggle' && video.paused) void video.play().catch(() => {});
        else video.pause();
        if (message.type === 'pause' && document.pictureInPictureElement === video) void document.exitPictureInPicture().catch(() => {});
      }
      report(true); reply({ ok: true });
    } else if (message.type === 'transcript' && message.videoId === id()) {
      void transcript().then(reply, () => reply({ error: 'No se pudo leer la transcripción.' }));
      return true;
    } else if (message.type === 'refresh') { bind(); reply({ ok: true }); }
  });
  document.addEventListener('yt-navigate-finish', bind);
  document.addEventListener('visibilitychange', () => report(true));
  window.addEventListener('pagehide', () => chrome.runtime.sendMessage({ type: 'snapshot', snapshot: null }).catch(() => {}));
  new MutationObserver(() => { if (document.querySelector('video') !== bound) bind(); })
    .observe(document.documentElement, { childList: true, subtree: true });
  bind();
})();
