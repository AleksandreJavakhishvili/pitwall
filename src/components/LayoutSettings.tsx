import { useActions } from "../lib/actions";
import { DEFAULT_FONT, DENSITIES, DENSITY_CELLS, DENSITY_LABEL, FONT_FLOOR, FONT_MAX, FONT_MIN } from "../layout/density";
import { keys } from "../lib/host";

/** Settings → Appearance: global tile density and the base terminal font. */
export function LayoutSettings() {
  const { layoutPrefs, setDensity, setBaseFont } = useActions();
  const { density, fontSize } = layoutPrefs;
  return (
    <div className="setting">
      <div className="setting-row">
        <span className="muted">Density</span>
        <span className="spacer" />
        <div className="seg" role="radiogroup" aria-label="Tile density">
          {DENSITIES.map((d) => (
            <button
              key={d}
              role="radio"
              aria-checked={d === density}
              className="seg-btn"
              data-on={d === density}
              onClick={() => setDensity(d)}
              title={`Smallest tile ${DENSITY_CELLS[d].cols} cols × ${DENSITY_CELLS[d].rows} rows`}
            >
              {DENSITY_LABEL[d]}
            </button>
          ))}
        </div>
      </div>
      <div className="setting-row">
        <span className="muted">Terminal font</span>
        <span className="spacer" />
        <button className="small-btn" onClick={() => setBaseFont(fontSize - 1)} disabled={fontSize <= FONT_MIN} aria-label="Smaller">
          −
        </button>
        <span className="mono font-size-val">{fontSize}px</span>
        <button className="small-btn" onClick={() => setBaseFont(fontSize + 1)} disabled={fontSize >= FONT_MAX} aria-label="Larger">
          +
        </button>
        {fontSize !== DEFAULT_FONT && (
          <button className="ghost-btn" onClick={() => setBaseFont(null)}>
            Reset
          </button>
        )}
      </div>
      <p className="hint">
        Density is the smallest tile before extra agents fold into chips (overridable per space in its bar or {keys("⌘K")}). Tiles shrink their
        font down to {FONT_FLOOR}px first; {keys("⌘+ / ⌘− / ⌘0")} change the focused tile only.
      </p>
    </div>
  );
}
