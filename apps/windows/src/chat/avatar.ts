// Buddy's face for the chat: the avatar square the core gives with the character, cut from idle frame 0.
import { invoke } from "@tauri-apps/api/core";
import { argbToRgba, type Sprite } from "../sprite";

let face: Promise<string | null> | null = null;

/** A data URL of Buddy's face (pixelated by CSS), made once. */
export function buddyFace(): Promise<string | null> {
  face ??= invoke<Sprite & { face?: { x: number; y: number; size: number } | null }>("sprite", { id: "buddy-base" })
    .then((sprite) => {
      const f = sprite.face;
      const idle = sprite.states.find((s) => s.name === "idle")?.frames[0];
      if (!f || !idle) return null;
      const full = document.createElement("canvas");
      full.width = full.height = sprite.size;
      full.getContext("2d")!.putImageData(new ImageData(argbToRgba(idle), sprite.size, sprite.size), 0, 0);
      const out = document.createElement("canvas");
      out.width = out.height = f.size;
      out.getContext("2d")!.drawImage(full, f.x, f.y, f.size, f.size, 0, 0, f.size, f.size);
      return out.toDataURL("image/png");
    })
    .catch(() => null);
  return face;
}
