import { keys } from "../lib/host";

/** A shortcut, written the macOS way ("⌘K") and shown as this desktop types it. `submit`: a form's ⌘↵ (Ctrl+↵ elsewhere). */
export function Kbd({ children, submit }: { children: string; submit?: boolean }) {
  return <kbd className="kbd">{keys(children, { submit })}</kbd>;
}
