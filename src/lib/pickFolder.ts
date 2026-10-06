import { errorText, inTauri } from "../api";

/** Whether a native folder picker exists (not in the browser mock). */
export const canPickFolder = inTauri;

/**
 * Native macOS folder picker (tauri-plugin-dialog).
 * Returns the chosen absolute path, or null when cancelled, unavailable
 * (mock mode) or failed — callers keep their typed-path field as fallback.
 */
export async function pickFolder(title = "Choose a project folder", defaultPath?: string): Promise<string | null> {
  if (!canPickFolder) return null;
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({ directory: true, multiple: false, title, defaultPath: defaultPath || undefined });
    return typeof picked === "string" ? picked : null;
  } catch (e) {
    console.error("pitwall: folder picker failed:", errorText(e));
    return null;
  }
}
