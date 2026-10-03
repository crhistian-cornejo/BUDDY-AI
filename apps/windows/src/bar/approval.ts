// What an approval card needs before «Permitir» may be clicked: the whole command must have been in view.

/** The scroll box of the command. */
export interface Box { scrollTop: number; clientHeight: number; scrollHeight: number }

/** True when nothing of the command is left below the box: it fits, or the user scrolled to its end. A box with no
 *  size yet (not laid out) has shown nothing. */
export function seenWhole(box: Box): boolean {
  return box.clientHeight > 0 && box.scrollTop + box.clientHeight >= box.scrollHeight - 2;
}

/** Keeps `buttons` (the ones that allow) off, and `hint` showing, until the whole of `pre` has been in view.
 *  `done` runs once, when it has. */
export function lockUntilRead(pre: HTMLElement, buttons: HTMLButtonElement[], hint: HTMLElement, done: () => void) {
  let read = false;
  const set = (locked: boolean) => {
    for (const b of buttons) b.disabled = locked;
    hint.hidden = !locked;
  };
  const check = () => {
    if (read || !seenWhole(pre)) return;
    read = true;
    set(false);
    watcher.disconnect();
    done();
  };
  const watcher = new ResizeObserver(check);
  set(true);
  watcher.observe(pre);
  pre.addEventListener("scroll", check, { passive: true });
  check();
}
