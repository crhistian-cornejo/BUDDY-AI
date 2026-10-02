import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import tokens, { applyTokens } from "./tokens";
import { type Sprite } from "./sprite";
import { Player, sleep } from "./player";

applyTokens();

interface PetPlan {
  waitMs: number;
  state: string;
  durationMs: number;
  dx: number;
}

const canvas = document.getElementById("pet") as HTMLCanvasElement;
const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
let player: Player;
/** Bumped to end the idle loop (drag, click); a new loop starts afterwards. */
let lifeToken = 0;

async function main(): Promise<void> {
  const sprite = await invoke<Sprite>("sprite", { id: "buddy-base" });
  const scale = tokens.pet.scaleNormal;
  canvas.style.width = `${sprite.size * scale}px`;
  canvas.style.height = `${sprite.size * scale}px`;
  player = new Player(canvas, sprite, tokens.motion.maxFps);
  await player.play("wave", 1);
  void life();
}

/** Ask the core what to do, wait, play it, repeat. Timers sleep in between. */
async function life(): Promise<void> {
  const token = ++lifeToken;
  while (token === lifeToken) {
    const plan = await invoke<PetPlan>("pet_next", { reduceMotion: reduceMotion.matches });
    await sleep(plan.waitMs);
    if (token !== lifeToken) return;
    const seconds = plan.durationMs / 1000;
    if (plan.dx !== 0 && seconds > 0) {
      const speed = plan.dx / seconds;
      await player.play(plan.state, seconds, (interval) => invoke("pet_step", { dx: speed * interval }));
    } else {
      await player.play(plan.state, seconds);
    }
  }
}

// Click or drag: past 3 px of movement it is a drag (native, so it is smooth); otherwise a click.
canvas.addEventListener("mousedown", (down) => {
  if (down.button !== 0) return;
  const move = (e: MouseEvent) => {
    if (Math.hypot(e.screenX - down.screenX, e.screenY - down.screenY) <= 3) return;
    cleanup();
    lifeToken++;
    void player.loop("drag");
    void getCurrentWindow()
      .startDragging()
      .finally(async () => {
        player.stop();
        player.show("idle");
        await invoke("pet_settle");
        void life();
      });
  };
  const up = () => {
    cleanup();
    void invoke("toggle_chat");
  };
  const cleanup = () => {
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", up);
  };
  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", up);
});

window.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  void invoke("show_pet_menu");
});

// Agent states from the core: think/work/ask loop until the next state; done and error play once.
let activity: string | null = null;
void listen<{ type: string; state?: string }>("core-event", ({ payload }) => {
  if (payload.type !== "mascotState" || !player) return;
  const state = payload.state ?? "idle";
  if (["think", "work", "ask", "listen"].includes(state)) {
    activity = state;
    lifeToken++;
    void player.loop(state);
  } else {
    const wasBusy = activity !== null;
    activity = null;
    player.stop();
    player.show("idle");
    const reaction = state === "done" || state === "error" ? player.play(state, 1) : Promise.resolve(true);
    if (wasBusy) void reaction.then(() => life());
  }
});

void main();
