import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The React UI, built as the website's live demo (website/scripts/build-demo.mjs)
// or served with `pnpm dev`; it always runs against the in-browser mock
// (src/README.md).
// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react()],

  // Pre-bundle the Review diff editor (CodeMirror) at dev-server start. Otherwise
  // Vite discovers it the first time Review opens, re-optimizes and
  // force-reloads the page mid-import, which left it blank.
  optimizeDeps: {
    include: [
      "@codemirror/commands",
      "@codemirror/lang-markdown",
      "@codemirror/language",
      "@codemirror/language-data",
      "@codemirror/merge",
      "@codemirror/search",
      "@codemirror/state",
      "@codemirror/view",
      "@lezer/highlight",
      "@replit/codemirror-indentation-markers",
    ],
  },

  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
}));
