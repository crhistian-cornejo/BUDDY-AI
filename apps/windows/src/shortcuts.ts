import { invoke } from "@tauri-apps/api/core";

// Local to Buddy's focused windows; no system-wide keyboard hook.
window.addEventListener("keydown", (event) => {
  if (!event.ctrlKey || event.altKey || event.metaKey || event.repeat) return;
  const key = event.key.toLowerCase();
  if (event.shiftKey && key === "p") {
    event.preventDefault();
    void invoke<boolean>("setting_flag", { key: "pet.wander" })
      .then((on) => invoke("set_setting_flag", { key: "pet.wander", on: !on }));
  } else if (!event.shiftKey && key === ",") {
    event.preventDefault();
    void invoke("open_settings");
  } else if (!event.shiftKey && key === "q") {
    event.preventDefault();
    void invoke("quit_app");
  }
});
