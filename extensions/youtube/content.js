// Local playback state only. Never access a media URL, account, cookie or private YouTube API.
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
  chrome.runtime.onMessage.addListener((message, _sender, reply) => {
    if (['pause', 'toggle'].includes(message.type) && message.videoId === id()) {
      const video = document.querySelector('video');
      if (video) {
        if (message.type === 'toggle' && video.paused) void video.play().catch(() => {});
        else video.pause();
        if (message.type === 'pause' && document.pictureInPictureElement === video) void document.exitPictureInPicture().catch(() => {});
      }
      report(true); reply({ ok: true });
    } else if (message.type === 'refresh') { bind(); reply({ ok: true }); }
  });
  document.addEventListener('yt-navigate-finish', bind);
  document.addEventListener('visibilitychange', () => report(true));
  window.addEventListener('pagehide', () => chrome.runtime.sendMessage({ type: 'snapshot', snapshot: null }).catch(() => {}));
  new MutationObserver(() => { if (document.querySelector('video') !== bound) bind(); })
    .observe(document.documentElement, { childList: true, subtree: true });
  bind();
})();
