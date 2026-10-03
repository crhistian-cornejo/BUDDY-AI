const status = document.getElementById('status'), feedback = document.getElementById('feedback');
const enable = document.getElementById('enable'), revoke = document.getElementById('revoke');
let activeTab, origin;
document.getElementById('version').textContent = chrome.runtime.getManifest().version;
async function check() {
  const state = await chrome.runtime.sendMessage({ type: 'status' }).catch(() => null);
  status.textContent = state?.connection || 'Abre Buddy y vuelve a intentarlo.';
  status.classList.toggle('connected', Boolean(state?.connection?.includes('Conectado')));
}
function scriptId() { return 'buddy-' + new URL(origin).hostname.replace(/[^a-z0-9]/gi, '-'); }
async function siteState() {
  const allowed = origin === 'https://www.youtube.com' || await chrome.permissions.contains({ origins: [origin + '/*'] });
  enable.textContent = allowed ? 'Detección activada ✓' : 'Activar este sitio';
  enable.disabled = allowed; revoke.hidden = !allowed || origin === 'https://www.youtube.com';
}
enable.addEventListener('click', async () => {
  try {
    // Permission request stays directly within this explicit click, for this origin alone.
    if (!await chrome.permissions.request({ origins: [origin + '/*'] })) return;
    const id = scriptId();
    await chrome.scripting.unregisterContentScripts({ ids: [id] }).catch(() => {});
    await chrome.scripting.registerContentScripts([{ id, matches: [origin + '/*'], js: ['browser-video.js'], allFrames: true, persistAcrossSessions: true }]);
    await chrome.scripting.executeScript({ target: { tabId: activeTab.id, allFrames: true }, files: ['browser-video.js'] });
    feedback.textContent = 'Listo. Este sitio puede mostrar su video en Buddy.'; await siteState();
  } catch (_) { feedback.textContent = 'Recarga esta pestaña para activar la detección.'; }
});
revoke.addEventListener('click', async () => {
  await chrome.scripting.unregisterContentScripts({ ids: [scriptId()] }).catch(() => {});
  await chrome.permissions.remove({ origins: [origin + '/*'] });
  feedback.textContent = 'Detección desactivada. Recarga la pestaña.'; await siteState();
});
document.getElementById('pip').addEventListener('click', async () => {
  try {
    const results = await chrome.scripting.executeScript({ target: { tabId: activeTab.id }, func: async () => {
      const video = [...document.querySelectorAll('video')].sort((a, b) => Number(!b.paused) - Number(!a.paused) || b.clientWidth * b.clientHeight - a.clientWidth * a.clientHeight)[0];
      if (!video || video.readyState === 0) return 'No hay un video listo en esta pestaña.';
      if (!document.pictureInPictureEnabled || video.disablePictureInPicture) return 'Esta página no permite video flotante.';
      try { if (document.pictureInPictureElement === video) await document.exitPictureInPicture(); else await video.requestPictureInPicture(); return 'Listo. Puedes mover el video y seguir chateando con Buddy.'; }
      catch (_) { return 'La página bloqueó el modo flotante. Usa su botón propio de imagen en imagen si está disponible.'; }
    } });
    feedback.textContent = results[0]?.result || 'Este reproductor no es compatible.';
  } catch (_) { feedback.textContent = 'No se puede abrir el video de esta pestaña. Prueba desde el reproductor del sitio.'; }
});
for (const [id, destination] of [['companion', 'floating'], ['notch', 'notch']]) {
  document.getElementById(id).addEventListener('click', async () => {
    const result = await chrome.runtime.sendMessage({ type: 'open-companion', destination }).catch(() => null);
    feedback.textContent = result?.ok ? 'Abriendo el video en Buddy…' : 'Abre Buddy y espera a que detecte el video de YouTube.';
  });
}
document.getElementById('check').addEventListener('click', () => void check());
void check();
void chrome.tabs.query({ active: true, currentWindow: true }).then(async tabs => {
  activeTab = tabs[0];
  try { const u = new URL(activeTab.url); if (u.protocol !== 'https:') throw Error(); origin = u.origin; document.getElementById('site').textContent = u.hostname; await siteState(); document.getElementById('youtube-destinations').hidden = origin !== 'https://www.youtube.com'; }
  catch (_) { enable.disabled = true; document.getElementById('pip').disabled = true; document.getElementById('site').textContent = 'Abre una pestaña con un video'; }
});
