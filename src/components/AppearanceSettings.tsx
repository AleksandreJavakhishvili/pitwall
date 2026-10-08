import { useActions } from "../lib/actions";
import { host } from "../lib/host";
import { glassLabel, LOOKS, LOOK_LABEL } from "../lib/look";
import { THEME_LABEL, THEME_PREFS } from "../lib/theme";

const GLASS_HINT = {
  vibrancy: "Translucent chrome over the window's native material. Terminals stay solid.",
  mica: "Translucent chrome over Windows 11 Mica. Terminals stay solid.",
  none: "Tinted chrome over Pitwall's own backdrop, no blur (no window material on this desktop). Terminals stay solid.",
} as const;

/** Settings → Appearance: theme, look and motion, shared by every window. */
export function AppearanceSettings() {
  const { theme, setTheme, look, setLook, reduceMotion, setReduceMotion } = useActions();
  const offer = host().glass;
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
      <div className="setting-row">
        <span className="muted">Look</span>
        <span className="spacer" />
        <div className="seg" role="radiogroup" aria-label="Look">
          {LOOKS.map((l) => (
            <button
              key={l}
              role="radio"
              aria-checked={l === look}
              className="seg-btn"
              data-on={l === look}
              onClick={() => setLook(l)}
              title={l === "glass" ? GLASS_HINT[offer] : undefined}
            >
              {l === "glass" ? glassLabel(offer) : LOOK_LABEL[l]}
            </button>
          ))}
        </div>
      </div>
      <div className="setting-row">
        <span className="muted">Reduce motion</span>
        <span className="spacer" />
        <label className="toggle" title="Fewer animations (the system setting also applies)">
          <input type="checkbox" checked={reduceMotion} onChange={(e) => setReduceMotion(e.target.checked)} />
          <span className="toggle-track" aria-hidden />
        </label>
      </div>
    </div>
  );
}
