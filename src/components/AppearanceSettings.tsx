import { useActions } from "../lib/actions";
import { THEME_LABEL, THEME_PREFS } from "../lib/theme";

/** Settings → Appearance: System / Dark / Light, shared by every window. */
export function AppearanceSettings() {
  const { theme, setTheme } = useActions();
  return (
    <div className="setting">
      <div className="setting-head">
        <span className="label">Appearance</span>
      </div>
      <div className="setting-row">
        <span className="muted">Theme</span>
        <span className="spacer" />
        <div className="seg" role="radiogroup" aria-label="Theme">
          {THEME_PREFS.map((t) => (
            <button
              key={t}
              role="radio"
              aria-checked={t === theme}
              className="seg-btn"
              data-on={t === theme}
              onClick={() => setTheme(t)}
              title={t === "system" ? "Follow macOS" : undefined}
            >
              {THEME_LABEL[t]}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
