// assets/design-tokens.json, shared with the Mac app. The Mac uses the system's own colours; Windows follows the
// system theme with the token palettes (`color` for dark, `colorLight` for light).
import tokens from "../../../assets/design-tokens.json";

export type Tokens = typeof tokens;
export default tokens;

const dark = window.matchMedia("(prefers-color-scheme: dark)");

/** Exposes the tokens as CSS custom properties (--color-surface, --radius-card…) and follows theme changes. */
export function applyTokens(root: HTMLElement = document.documentElement): void {
  const paint = () => {
    const palette = dark.matches ? tokens.color : tokens.colorLight;
    for (const [name, value] of Object.entries(palette)) root.style.setProperty(`--color-${kebab(name)}`, value);
    root.style.colorScheme = dark.matches ? "dark" : "light";
  };
  paint();
  dark.addEventListener("change", paint);
  for (const [name, value] of Object.entries(tokens.radius)) root.style.setProperty(`--radius-${kebab(name)}`, `${value}px`);
  root.style.setProperty("--font-size-body", `${tokens.font.sizeBody + 1}px`);
  root.style.setProperty("--fade-out", `${tokens.motion.fadeOut}s`);
}

function kebab(name: string): string {
  return name.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);
}
