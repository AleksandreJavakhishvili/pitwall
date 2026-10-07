export function DiffStat({ added, removed }: { added: number; removed: number }) {
  if (!added && !removed) return <span className="diffstat diffstat-empty">±0</span>;
  return (
    <span className="diffstat">
      <span className="add">+{added}</span>
      <span className="del">−{removed}</span>
    </span>
  );
}
