import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import tokens, { applyTokens } from "./tokens";

applyTokens();

const bubble = document.getElementById("bubble")!;

async function main(): Promise<void> {
  bubble.textContent = await invoke<string>("hello");
  requestAnimationFrame(() => bubble.classList.add("shown"));
  window.setTimeout(() => {
    bubble.classList.remove("shown");
    window.setTimeout(() => void getCurrentWindow().close(), tokens.motion.fadeOut * 1000 + 50);
  }, Math.max(tokens.motion.helloSeconds, bubble.textContent.length / 10) * 1000);
}

void main();
