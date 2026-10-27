import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri から開く開発サーバー。ポートは tauri.conf.json の devUrl と揃える
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
});
