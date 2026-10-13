import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { App } from "./App";
import { About } from "./components/About";
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

  const root = document.getElementById("root");
  if (root) {
    // The same page runs in every window: main, the dictation indicator and
    // About.
    const label = getCurrentWindow().label;
    const page = label === "indicator" ? <Indicator /> : label === "about" ? <About /> : <App />;
    createRoot(root).render(<StrictMode>{page}</StrictMode>);
  }
}

void start();
