import { resolve } from "node:path";
import { defineConfig } from "vite";

// Two pages: the mascot and its speech bubble (a separate click-through window).
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // assets/design-tokens.json lives at the repository root, shared with the Mac app.
    fs: { allow: [resolve(__dirname, "../..")] },
  },
  build: {
    target: "es2022",
    rollupOptions: {
      input: {
        pet: resolve(__dirname, "index.html"),
        bubble: resolve(__dirname, "bubble.html"),
        chat: resolve(__dirname, "chat.html"),
        history: resolve(__dirname, "history.html"),
        bar: resolve(__dirname, "bar.html"),
      },
    },
  },
});
