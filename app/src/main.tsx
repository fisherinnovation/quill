import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, applyTheme, events } from "./api";
import { App } from "./App";
import { About } from "./components/About";
import { CallPromptCard } from "./components/CallPrompt";
import { Indicator } from "./components/Indicator";
import "./styles.css";

async function start() {
  // Outside Tauri (a plain browser on the dev server), fake the Rust side
  // with sample data so the UI can be previewed. Stripped from release builds.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const { installMocks } = await import("./dev/mock");
    installMocks();
  }
  // Lets CSS adapt to the platform (e.g. room for the macOS window buttons).
  document.documentElement.dataset.platform = navigator.userAgent.includes("Mac") ? "mac" : "other";
  // The theme before the first render, so the page doesn't flash the OS's
  // one first; then follow changes made in any window.
  await api
    .getSettings()
    .then((s) => applyTheme(s.theme))
    .catch(() => {});
  void events.onTheme(applyTheme);

  const root = document.getElementById("root");
  if (root) {
    // The same page runs in every window: main, the dictation indicator,
    // the call prompt and About.
    const label = getCurrentWindow().label;
    const page =
      label === "indicator" ? (
        <Indicator />
      ) : label === "prompt" ? (
        <CallPromptCard />
      ) : label === "about" ? (
        <About />
      ) : (
        <App />
      );
    createRoot(root).render(<StrictMode>{page}</StrictMode>);
  }
}

void start();
