import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react()],

  // Pre-bundle Monaco at dev-server start. Otherwise Vite discovers it the first
  // time Review opens, re-optimizes and force-reloads the webview mid-import,
  // which left the window blank.
  optimizeDeps: {
    include: [
      "@monaco-editor/react",
      "monaco-editor/editor/editor.api",
      "monaco-editor/basic-languages/monaco.contribution",
      "monaco-editor/features/bracketMatching/register",
      "monaco-editor/features/clipboard/register",
      "monaco-editor/features/codicon/register",
      "monaco-editor/features/contextmenu/register",
      "monaco-editor/features/diffEditor/register",
      "monaco-editor/features/find/register",
      "monaco-editor/features/folding/register",
      "monaco-editor/features/hover/register",
      "monaco-editor/features/readOnlyMessage/register",
    ],
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
