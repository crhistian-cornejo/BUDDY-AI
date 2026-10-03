// Isolated wrapper for the official player. It has no Tauri commands or access to Buddy data.
const [id, rawSeconds] = location.hash.slice(1).split('/');
const seconds = Number(rawSeconds);
let tick: ReturnType<typeof setInterval> | undefined;
window.addEventListener('pagehide', () => clearInterval(tick));
const report = (type: string, code = 0) => parent.postMessage({ channel: 'buddy-youtube', videoId: id, type, code }, location.origin);
if (/^[\w-]{11}$/.test(id ?? '') && Number.isFinite(seconds) && seconds >= 0 && seconds <= 604800) {
  type PlayerOptions = { width: string; height: string; videoId: string; playerVars: Record<string, string | number>; events: Record<string, (event: { data: number; target: { getCurrentTime(): number } }) => void> };
  const ytWindow = window as unknown as { onYouTubeIframeAPIReady: () => void; YT: { Player: new (element: string, options: PlayerOptions) => unknown } };
  ytWindow.onYouTubeIframeAPIReady = () => {
    new ytWindow.YT.Player('player', {
      width: '100%', height: '100%', videoId: id!,
      playerVars: { autoplay: 1, controls: 1, playsinline: 1, start: Math.floor(seconds), origin: location.origin, widget_referrer: 'https://io.github.crhistian-cornejo.buddy' },
      events: { onStateChange: event => { clearInterval(tick); report('position', Math.floor(event.target.getCurrentTime())); if (event.data === 1) { report('playing'); tick = setInterval(() => report('position', Math.floor(event.target.getCurrentTime())), 1000); } }, onError: event => report('error', event.data), onAutoplayBlocked: () => report('blocked') },
    });
  };
  const script = document.createElement('script'); script.src = 'https://www.youtube.com/iframe_api'; script.onerror = () => report('error'); document.head.append(script);
}
