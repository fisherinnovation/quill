import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
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
    createRoot(root).render(
      <StrictMode>
        <App />
      </StrictMode>,
    );
  }
}

void start();
