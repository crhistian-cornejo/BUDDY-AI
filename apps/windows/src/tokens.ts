// assets/design-tokens.json, shared with the Mac app.
import tokens from "../../../assets/design-tokens.json";

export type Tokens = typeof tokens;
export default tokens;

/** Exposes the tokens as CSS custom properties (--color-surface, --radius-bubble, --font-size-body…). */
export function applyTokens(root: HTMLElement = document.documentElement): void {
  for (const [name, value] of Object.entries(tokens.color)) root.style.setProperty(`--color-${kebab(name)}`, value);
  for (const [name, value] of Object.entries(tokens.radius)) root.style.setProperty(`--radius-${kebab(name)}`, `${value}px`);
  root.style.setProperty("--font-size-body", `${tokens.font.sizeBody}px`);
  root.style.setProperty("--fade-out", `${tokens.motion.fadeOut}s`);
}

function kebab(name: string): string {
  return name.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);
}
