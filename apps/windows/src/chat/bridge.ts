import { invoke } from "@tauri-apps/api/core";

export const IS_TAURI = "__TAURI_INTERNALS__" in window;

/** What the answer views ask of the app. The core validates every address before anything opens. */
export const Bridge = {
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  /** Site icons are not fetched yet (a later phase): letter tiles only. */
  sourceIcon: (_url: string) => Promise.resolve<string | null>(null),
};
