import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import tokens, { applyTokens } from "./tokens";
import { type Sprite } from "./sprite";
import { Player, sleep } from "./player";
import "./shortcuts";

applyTokens();

interface PetPlan {
  waitMs: number;
  state: string;
  durationMs: number;
  dx: number;
  /** A short transition played once first: "sit-down" or "stand-up" ("" for none). */
  intro: string;
  /** The still frame held afterwards: "idle" or "sit". */
  rest: string;
}

const canvas = document.getElementById("pet") as HTMLCanvasElement;
const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
let player: Player;
/** Bumped to end the idle loop (drag, click); a new loop starts afterwards. */
let lifeToken = 0;
/** Last time Buddy was used (hover, click, drag, chat, work, talk, walk): the core sits it down 10 s later. */
let lastUse = Date.now();
/** Holding the seated frame (the last plan rested on "sit"). */
let seated = false;
let hovering = false;
/** The idle loop is waiting before its next plan (so a use can re-plan at once). */
let waiting = false;
/** An agent state (think, work, ask, listen) shown until the next state; the idle loop is stopped meanwhile. */
let activity: string | null = null;
/** Sitting at the laptop with the glasses on: thinking and working show these instead of the plain states. */
let atLaptop = false;
const LAPTOP_STATES: Record<string, string> = { think: "laptop-think", work: "laptop-type" };

/** Closes the laptop, takes the glasses off and stands up, if Buddy was at it. */
async function leaveLaptop(): Promise<boolean> {
  if (!atLaptop) return true;
  atLaptop = false;
  return player.has("laptop-off") ? player.play("laptop-off", 0) : true;
}

async function main(): Promise<void> {
  const sprite = await invoke<Sprite>("sprite", { id: "buddy-base" });
  const scale = tokens.pet.scaleNormal;
  canvas.style.width = `${sprite.size * scale}px`;
  canvas.style.height = `${sprite.size * scale}px`;
  player = new Player(canvas, sprite, tokens.motion.maxFps);
  await player.play("wave", 1);
  void life();
}

/** Ask the core what to do, wait, play it, repeat. Timers sleep in between. `standUp` first gets a seated Buddy up. */
async function life(standUp = false): Promise<void> {
  const token = ++lifeToken;
  waiting = false;
  if (standUp) {
    if (reduceMotion.matches) player.show("idle");
    else await player.play("stand-up", 0);
  }
  while (token === lifeToken) {
    const plan = await invoke<PetPlan>("pet_next", {
      reduceMotion: reduceMotion.matches,
      untouchedSeconds: (Date.now() - lastUse) / 1000,
      hovering,
      sitting: seated,
    });
    if (token !== lifeToken) return;
    waiting = true;
    await sleep(plan.waitMs);
    if (token !== lifeToken) return;
    waiting = false;
    const seconds = plan.durationMs / 1000;
    if (plan.dx !== 0 && seconds > 0) {
      if (plan.intro && !(await player.play(plan.intro, 0))) continue;
      seated = false;
      const speed = plan.dx / seconds;
      await player.play(plan.state, seconds, (interval) => invoke("pet_step", { dx: speed * interval }));
      if (token !== lifeToken) return;
      lastUse = Date.now();
    } else {
      if (plan.intro && !(await player.play(plan.intro, 0, undefined, plan.rest))) continue;
      const done = await player.play(plan.state, seconds, undefined, plan.rest);
      if (done && token === lifeToken) seated = plan.rest === "sit";
    }
  }
}

/** Buddy was used: the count to sitting restarts, a seated Buddy stands up and a waiting plan is made again. */
function used(standUp = true): void {
  lastUse = Date.now();
  if (!player || activity !== null) return;
  if (seated) {
    seated = false;
    void life(standUp);
  } else if (waiting) {
    void life();
  }
}

canvas.addEventListener("mouseenter", () => {
  hovering = true;
  used();
});
canvas.addEventListener("mouseleave", () => {
  hovering = false;
  lastUse = Date.now();
});
// The chat opened or closed, or Buddy talks (its bubble).
void listen("pet-used", () => used());

// Click or drag: past 3 px of movement it is a drag (native, so it is smooth); otherwise a click.
canvas.addEventListener("mousedown", (down) => {
  if (down.button !== 0) return;
  const move = (e: MouseEvent) => {
    if (Math.hypot(e.screenX - down.screenX, e.screenY - down.screenY) <= 3) return;
    cleanup();
    lifeToken++;
    seated = false;
    lastUse = Date.now();
    void player.loop("drag");
    void getCurrentWindow()
      .startDragging()
      .finally(async () => {
        player.stop();
        player.show("idle");
        lastUse = Date.now();
        await invoke("pet_settle");
        if (activity !== null) void player.loop(atLaptop ? (LAPTOP_STATES[activity] ?? activity) : activity);
        else void life();
      });
  };
  const up = () => {
    cleanup();
    used();
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

// Agent states from the core: think/work/ask loop until the next state (a seated Buddy stands up first); done and
// error play once.
void listen<{ type: string; state?: string }>("core-event", ({ payload }) => {
  if (payload.type !== "mascotState" || !player) return;
  const state = payload.state ?? "idle";
  const wasSeated = seated;
  if (["think", "work", "ask", "listen"].includes(state)) {
    activity = state;
    seated = false;
    lastUse = Date.now();
    lifeToken++;
    const laptop = LAPTOP_STATES[state];
    void (async () => {
      if (laptop && player.has(laptop)) {
        if (!atLaptop) {
          if (wasSeated && !reduceMotion.matches && !(await player.play("stand-up", 0))) return;
          atLaptop = true;
          if (!reduceMotion.matches && !(await player.play("laptop-on", 0, undefined, "laptop-type"))) return;
        }
        void player.loop(laptop);
        return;
      }
      if (!(await leaveLaptop())) return;
      if (wasSeated && !reduceMotion.matches && !(await player.play("stand-up", 0))) return;
      void player.loop(state);
    })();
  } else {
    const wasBusy = activity !== null;
    const reacts = state === "done" || state === "error";
    // An idle state while already idle changes nothing (a seated Buddy stays seated).
    if (!wasBusy && !reacts) return;
    activity = null;
    seated = false;
    lastUse = Date.now();
    player.stop();
    player.show(atLaptop ? "laptop-type" : "idle");
    const reaction = (async () => {
      if (!(await leaveLaptop())) return false;
      player.show("idle");
      return reacts ? player.play(state, 1) : true;
    })();
    // A plan made while seated (or the stopped loop) must not run: plan again once the reaction ends.
    if (wasBusy || wasSeated || waiting) {
      lifeToken++;
      void reaction.then(() => {
        if (activity === null) void life();
      });
    }
  }
});

void main();
