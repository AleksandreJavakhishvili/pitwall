/**
 * No native folder picker: this UI runs only as the website's demo, in a
 * browser (src/README.md). Callers keep their typed-path field.
 */
export const canPickFolder = false;

/** Always null (no picker in the browser demo). */
export async function pickFolder(_title?: string, _defaultPath?: string): Promise<string | null> {
  return null;
}
