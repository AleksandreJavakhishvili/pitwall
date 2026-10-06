// Tiny inline stroke icons (no icon font / network).
const PATHS: Record<string, string> = {
  sidebar: "M3 4.5h14v11H3zM7.5 4.5v11",
  panel: "M3 4.5h14v11H3zM12.5 4.5v11",
  search: "M8.5 14a5.5 5.5 0 1 1 0-11 5.5 5.5 0 0 1 0 11zM12.5 12.5 17 17",
  gear: "M10 12.6a2.6 2.6 0 1 0 0-5.2 2.6 2.6 0 0 0 0 5.2zM10 2.5v2M10 15.5v2M2.5 10h2M15.5 10h2M4.7 4.7l1.4 1.4M13.9 13.9l1.4 1.4M4.7 15.3l1.4-1.4M13.9 6.1l1.4-1.4",
  plus: "M10 4v12M4 10h12",
  x: "M5 5l10 10M15 5 5 15",
  send: "M4 10h10M10 5l5 5-5 5",
  stop: "M6 6h8v8H6z",
  restart: "M4.5 10a5.5 5.5 0 1 0 1.7-4M4 3.5v3h3",
  trash: "M4.5 6h11M8 6V4.5h4V6M6 6l.7 10h6.6L14 6",
  branch: "M6 4v12M6 9c0-2 1.5-3 4-3h1M13 4a2 2 0 1 1 0 4 2 2 0 0 1 0-4zM6 16",
  folder: "M3 5.5h5l1.5 1.5H17v8.5H3z",
  chevron: "M8 5l5 5-5 5",
  grid: "M3.5 3.5h5.5v5.5H3.5zM11 3.5h5.5v5.5H11zM3.5 11h5.5v5.5H3.5zM11 11h5.5v5.5H11z",
  wall: "M2.5 4h4.5v5H2.5zM7.75 4h4.5v5h-4.5zM13 4h4.5v5H13zM2.5 11h4.5v5H2.5zM7.75 11h4.5v5h-4.5zM13 11h4.5v5H13z",
  maximize: "M4 8V4h4M16 8V4h-4M4 12v4h4M16 12v4h-4",
  restore: "M8 4v4H4M12 4v4h4M8 16v-4H4M12 16v-4h4",
  window: "M3 5h11v10H3zM6 5V3h11v10h-3",
  more: "M5 10h.01M10 10h.01M15 10h.01",
  review: "M4 3.5h8l4 4v9H4zM12 3.5v4h4M7 11h6M7 14h4",
};

export function Icon({ name, size = 16 }: { name: keyof typeof PATHS | string; size?: number }) {
  return (
    <svg className="icon" width={size} height={size} viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d={PATHS[name] ?? ""} />
    </svg>
  );
}
