import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import tokens, { applyTokens } from "./tokens";
import { argbToRgba, breathFrames, type Sprite } from "./sprite";

applyTokens();

const canvas = document.getElementById("pet") as HTMLCanvasElement;
const ctx = canvas.getContext("2d")!;
const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

async function main(): Promise<void> {
  const sprite = await invoke<Sprite>("sprite", { id: "buddy-base" });
  const scale = tokens.pet.scaleNormal;
  canvas.width = sprite.size;
  canvas.height = sprite.size;
  canvas.style.width = `${sprite.size * scale}px`;
  canvas.style.height = `${sprite.size * scale}px`;
  ctx.imageSmoothingEnabled = false;

  const idle = sprite.states.find((s) => s.name === "idle") ?? sprite.states[0];
  const frames = idle.frames.map((f) => new ImageData(argbToRgba(f), sprite.size, sprite.size));
  const show = (i: number) => ctx.putImageData(frames[i], 0, 0);
  show(0);

  // Idle: a still frame; every 6–10 s a ~2 s breath. Timers sleep in between, and reduced motion never moves.
  const rate = Math.max(1, Math.min(idle.fps, tokens.motion.maxFps));
  const breathe = () => {
    const { breathEveryMin, breathEveryMax } = tokens.motion;
    const wait = breathEveryMin + Math.random() * (breathEveryMax - breathEveryMin);
    window.setTimeout(() => {
      const sequence = reduceMotion.matches ? [] : breathFrames(frames.length, idle.fps, tokens.motion);
      sequence.forEach((frame, i) => window.setTimeout(() => show(frame), (i * 1000) / rate));
      window.setTimeout(breathe, (sequence.length * 1000) / rate);
    }, wait * 1000);
  };
  breathe();
}

// Drag with the left button; the native menu with the right one.
canvas.addEventListener("mousedown", (e) => {
  if (e.button === 0) void getCurrentWindow().startDragging();
});
window.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  void invoke("show_pet_menu");
});

void main();
