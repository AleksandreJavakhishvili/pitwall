import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { languageFor, type Lang } from "../review/editorSetup";
import { FileViewer, type MenuRequest } from "../review/diffView";
import { DiffMenu, useScheme } from "../review/ReviewDiff";

export interface Reveal {
  line: number;
  /** 0-based columns on that line to select (a search hit). */
  from?: number;
  to?: number;
  nonce: number;
}

/** One file, read-only, in Review's CodeMirror setup (lazy chunk shared with Review). */
export function FileEditor({ path, text, reveal }: { path: string; text: string; reveal: Reveal | null }) {
  const scheme = useScheme();
  const boxRef = useRef<HTMLDivElement | null>(null);
  const viewer = useRef<FileViewer | null>(null);
  const pending = useRef<Reveal | null>(null);
  const [lang, setLang] = useState<{ path: string; lang: Lang | null } | null>(null);
  const [menu, setMenu] = useState<MenuRequest | null>(null);

  useEffect(() => {
    let alive = true;
    void languageFor(path).then((l) => alive && setLang({ path, lang: l }));
    return () => {
      alive = false;
    };
  }, [path]);

  const show = (r: Reveal) => {
    if (viewer.current?.reveal(r.line, r.from, r.to)) pending.current = null;
    else pending.current = r;
  };

  const ready = lang?.path === path;
  useLayoutEffect(() => {
    const box = boxRef.current;
    if (!box || !ready) return;
    const v = new FileViewer({ parent: box, doc: text, lang: lang!.lang, onMenu: setMenu });
    viewer.current = v;
    if (pending.current) show(pending.current);
    return () => {
      viewer.current = null;
      setMenu(null);
      v.destroy();
    };
  }, [ready, text]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (reveal) show(reveal);
  }, [reveal?.nonce]); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div className="rv-diff ex-editor" data-scheme={scheme}>
      {!ready && <p className="hint pad">Loading…</p>}
      <div className="rv-diffbox" ref={boxRef} />
      {menu && <DiffMenu req={menu} onComment={null} onClose={() => setMenu(null)} />}
    </div>
  );
}

export default FileEditor;
