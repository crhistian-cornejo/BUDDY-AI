import { argbToRgba, type Sprite } from "./sprite";

/** Plays the mascot's states on a canvas. Only frame changes draw; nothing runs between plays. */
export class Player {
  private states = new Map<string, { fps: number; frames: ImageData[] }>();
  private ctx: CanvasRenderingContext2D;
  private token = 0;
  state = "idle";

  constructor(canvas: HTMLCanvasElement, sprite: Sprite, maxFps: number) {
    canvas.width = sprite.size;
    canvas.height = sprite.size;
    this.ctx = canvas.getContext("2d")!;
    this.ctx.imageSmoothingEnabled = false;
    for (const s of sprite.states) {
      const frames = s.frames.map((f) => new ImageData(argbToRgba(f), sprite.size, sprite.size));
      this.states.set(s.name, { fps: Math.min(s.fps, maxFps), frames });
    }
    this.show("idle");
  }

  has(name: string): boolean {
    return this.states.has(name);
  }

  show(name: string, frame = 0): void {
    const s = this.states.get(name) ?? this.states.get("idle")!;
    this.state = name;
    this.ctx.clearRect(0, 0, this.ctx.canvas.width, this.ctx.canvas.height);
    this.ctx.putImageData(s.frames[frame % s.frames.length], 0, 0);
  }

  /** Stops whatever is playing. */
  stop(): void {
    this.token++;
  }

  /** Plays `name` for `seconds` (at least one pass), calling `step` each frame; ends on idle unless stopped. */
  async play(name: string, seconds: number, step?: (interval: number) => void | Promise<void>): Promise<boolean> {
    const s = this.states.get(name);
    if (!s) return true;
    const token = ++this.token;
    const interval = 1 / s.fps;
    const count = Math.max(s.frames.length, Math.round(seconds / interval));
    for (let i = 0; i < count; i++) {
      if (token !== this.token) return false;
      this.show(name, i);
      await step?.(interval);
      await sleep(interval * 1000);
    }
    if (token !== this.token) return false;
    this.show("idle");
    return true;
  }

  /** Loops `name` until another play or stop. */
  async loop(name: string): Promise<void> {
    const s = this.states.get(name);
    if (!s) return;
    const token = ++this.token;
    for (let i = 0; token === this.token; i++) {
      this.show(name, i);
      if (s.frames.length === 1) return;
      await sleep(1000 / s.fps);
    }
  }
}

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}
