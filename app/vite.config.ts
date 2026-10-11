import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri loads the dev server in its window during `tauri dev`, and the built
// files from dist/ in release builds (see src-tauri/tauri.conf.json).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  // Tauri's webviews are modern (WebKit on macOS, WebView2 on Windows).
  build: { target: "es2022", outDir: "dist", emptyOutDir: true },
});
