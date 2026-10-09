// Tiny inline stroke icons (no icon font / network).
const PATHS: Record<string, string> = {
  sidebar: "M3 4.5h14v11H3zM7.5 4.5v11",
  panel: "M3 4.5h14v11H3zM12.5 4.5v11",
  search: "M8.5 14a5.5 5.5 0 1 1 0-11 5.5 5.5 0 0 1 0 11zM12.5 12.5 17 17",
  // Eight-tooth cog (the previous ray design read as a sun).
  gear: "M8.36 4.64 L8.81 2.49 L11.19 2.49 L11.64 4.64 L12.63 5.06 L14.47 3.85 L16.15 5.53 L14.94 7.37 L15.36 8.36 L17.51 8.81 L17.51 11.19 L15.36 11.64 L14.94 12.63 L16.15 14.47 L14.47 16.15 L12.63 14.94 L11.64 15.36 L11.19 17.51 L8.81 17.51 L8.36 15.36 L7.37 14.94 L5.53 16.15 L3.85 14.47 L5.06 12.63 L4.64 11.64 L2.49 11.19 L2.49 8.81 L4.64 8.36 L5.06 7.37 L3.85 5.53 L5.53 3.85 L7.37 5.06Z M12.4 10a2.4 2.4 0 1 1-4.8 0 2.4 2.4 0 0 1 4.8 0z",
  plus: "M10 4v12M4 10h12",
  x: "M5 5l10 10M15 5 5 15",
  send: "M4 10h10M10 5l5 5-5 5",
  stop: "M6 6h8v8H6z",
  restart: "M4.5 10a5.5 5.5 0 1 0 1.7-4M4 3.5v3h3",
  refresh: "M15.5 10a5.5 5.5 0 1 1-1.7-4M16 3.5v3h-3",
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
  // The Race Engineer (a headset).
  headset: "M4 12.5V10a6 6 0 0 1 12 0v2.5M4 11.5h2.5v4H5a1 1 0 0 1-1-1zM16 11.5h-2.5v4H15a1 1 0 0 0 1-1zM14.5 15.5c0 1.2-1.3 2-3.5 2",
  terminal: "M3 4.5h14v11H3zM6 8l2.5 2L6 12M10.5 12.5H14",
  file: "M5 3h6.5L15 6.5V17H5zM11.5 3v3.5H15",
  expand: "M11 4h5v5M16 4l-6 6M9 16H4v-5M4 16l6-6",
  copy: "M7.5 7.5h8.5v8.5H7.5zM4.5 12.5V4h8.5",
};

export function Icon({ name, size = 16 }: { name: keyof typeof PATHS | string; size?: number }) {
  return (
    <svg className="icon" width={size} height={size} viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d={PATHS[name] ?? ""} />
    </svg>
  );
}
