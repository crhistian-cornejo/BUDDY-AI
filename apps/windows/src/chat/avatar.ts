// The agents' faces for the chat and Settings: the avatar square the core gives with each sprite, cut from idle
// frame 0. Each agent wears its own face («cara»: colour, accessory, eyes); Buddy's default is buddy-base itself.
// Twin of Avatar / AgentAvatarView in apps/macos/Sources/Chat/Avatar.swift.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { argbToRgba, type Sprite } from "../sprite";

type FacedSprite = Sprite & { face?: { x: number; y: number; size: number } | null };

const faces = new Map<string, Promise<string | null>>();
let names: Promise<Map<string, string>> | null = null;

/** A data URL of the avatar square of a sprite's idle frame 0 (pixelated by CSS). */
export function faceUrl(sprite: FacedSprite): string | null {
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
}

/** An agent's face, made once (until `forgetFace`). */
export function agentFace(agentId: string): Promise<string | null> {
  let face = faces.get(agentId);
  if (!face) {
    face = invoke<FacedSprite>("agent_sprite", { agentId }).then(faceUrl).catch(() =>
      agentId === "buddy" ? invoke<FacedSprite>("sprite", { id: "buddy-base" }).then(faceUrl).catch(() => null) : null);
    faces.set(agentId, face);
  }
  return face;
}

/** Buddy's face. */
export function buddyFace(): Promise<string | null> {
  return agentFace("buddy");
}

/** The face of the agent with that name (the chat keeps names); Buddy's when unknown. */
export async function faceForName(name: string): Promise<string | null> {
  names ??= invoke<{ id: string; name: string }[]>("agents")
    .then((list) => new Map(list.map((a) => [a.name, a.id])))
    .catch(() => new Map<string, string>());
  const id = (await names).get(name) ?? (name === "Buddy" ? "buddy" : name.toLowerCase());
  return agentFace(id);
}

/** Forget a face (or all of them) after Settings changes it. */
export function forgetFace(agentId?: string): void {
  if (agentId) faces.delete(agentId);
  else { faces.clear(); names = null; }
}

// A face changed in Settings (another window): the next one drawn here is the new one.
void listen<{ type: string; key?: string }>("core-event", ({ payload }) => {
  const changed = payload.type === "settingChanged" ? /^agent\.(.+)\.cara$/.exec(payload.key ?? "") : null;
  if (changed) forgetFace(changed[1]);
}).catch(() => {});
