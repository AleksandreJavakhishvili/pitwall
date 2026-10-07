// A small Markdown → HTML converter for the subset CHANGELOG.md (git-cliff) and
// ROADMAP.md use: headings, paragraphs, "-" / "*" / "1." lists (one level, with
// indented continuation lines), fenced code, inline code, links, **bold**,
// *emphasis* and <kbd>. Other inline HTML is escaped; HTML comments are dropped.
// No dependency on purpose; if the files start needing more, swap in `marked`.

const esc = (s) => s.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");

/** Turns a heading's text into an id: "Recently shipped" → "recently-shipped". */
export function slug(text) {
  return text.toLowerCase().replace(/<[^>]+>/g, "").replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
}

/** Inline Markdown → HTML. `link(href)` maps each link target (default: as is). */
export function inline(text, link = (h) => h) {
  const held = [];
  const hold = (html) => `\u0000${held.push(html) - 1}\u0000`;
  let s = text.replace(/`([^`]+)`/g, (_, code) => hold(`<code>${esc(code)}</code>`));
  s = esc(s).replace(/&lt;(\/?)kbd&gt;/g, "<$1kbd>");
  s = s.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (_, label, href) => {
    const target = link(href.replaceAll("&amp;", "&"));
    return `<a href="${esc(target)}">${label}</a>`;
  });
  s = s.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
  s = s.replace(/(^|[\s(])\*([^*\s][^*]*?)\*(?=[\s).,;:!?]|$)/g, "$1<em>$2</em>");
  return s.replace(/\u0000(\d+)\u0000/g, (_, i) => held[Number(i)]);
}

/** Block Markdown → HTML. Heading ids get `idPrefix`, so repeated headings stay unique. */
export function render(md, { link, idPrefix = "" } = {}) {
  const lines = md.replace(/<!--[\s\S]*?-->/g, "").replace(/\r\n?/g, "\n").split("\n");
  const out = [];
  let para = [];
  let list = null; // { tag, items: string[] }

  const flushPara = () => {
    if (para.length) out.push(`<p>${inline(para.join(" "), link)}</p>`);
    para = [];
  };
  const flushList = () => {
    if (list) out.push(`<${list.tag}>\n${list.items.map((i) => `  <li>${inline(i, link)}</li>`).join("\n")}\n</${list.tag}>`);
    list = null;
  };
  const flush = () => (flushPara(), flushList());

  for (let n = 0; n < lines.length; n++) {
    const line = lines[n];
    let m;
    if ((m = line.match(/^```/))) {
      flush();
      const code = [];
      while (++n < lines.length && !/^```/.test(lines[n])) code.push(lines[n]);
      out.push(`<pre><code>${esc(code.join("\n"))}</code></pre>`);
    } else if ((m = line.match(/^(#{1,6})\s+(.*?)\s*#*\s*$/))) {
      flush();
      const level = m[1].length;
      out.push(`<h${level} id="${idPrefix}${slug(m[2])}">${inline(m[2], link)}</h${level}>`);
    } else if ((m = line.match(/^\s{0,3}([-*]|\d+\.)\s+(.*)$/))) {
      flushPara();
      const tag = /\d/.test(m[1]) ? "ol" : "ul";
      if (list && list.tag !== tag) flushList();
      list ??= { tag, items: [] };
      list.items.push(m[2]);
    } else if (list && /^\s{2,}\S/.test(line)) {
      list.items[list.items.length - 1] += " " + line.trim();
    } else if (!line.trim()) {
      flush();
    } else {
      flushList();
      para.push(line.trim());
    }
  }
  flush();
  return out.join("\n");
}

/**
 * Splits a document at its "## " headings: { title, intro, sections: [{ heading, body }] }.
 * `title` is the "# " heading, `intro` the Markdown between it and the first section.
 */
export function sections(md) {
  const parts = md.replace(/\r\n?/g, "\n").split(/^## /m);
  const head = parts.shift();
  const title = head.match(/^# (.*)$/m)?.[1]?.trim() ?? "";
  const intro = head.replace(/^# .*$/m, "").trim();
  return {
    title,
    intro,
    sections: parts.map((p) => {
      const nl = p.indexOf("\n");
      return { heading: (nl === -1 ? p : p.slice(0, nl)).trim(), body: nl === -1 ? "" : p.slice(nl + 1).trim() };
    }),
  };
}
