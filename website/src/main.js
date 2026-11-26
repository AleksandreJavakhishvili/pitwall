import "./site.css";

const root = document.documentElement;
const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)");
root.classList.add("js");

// ---------- motion: header glass, reveals, pausing what nobody can see ----------
// The header frosts in once the page scrolls (CSS fades the glass layer in).
let scrolled = null;
function onScroll() {
  const s = window.scrollY > 12;
  if (s === scrolled) return;
  scrolled = s;
  root.classList.toggle("scrolled", s);
}
addEventListener("scroll", onScroll, { passive: true });
onScroll();

// Background tab: stop the sky's loops.
document.addEventListener("visibilitychange", () => root.classList.toggle("motion-paused", document.hidden));

// Looping status animations pause while their block is off screen.
const motionIO =
  "IntersectionObserver" in window &&
  new IntersectionObserver((entries) => {
    for (const e of entries) e.target.classList.toggle("motion-off", !e.isIntersecting);
  });
if (motionIO) for (const el of document.querySelectorAll("[data-demo], [data-float], [data-files], [data-accounts], .board-wrap, .lanes")) motionIO.observe(el);

// The feature loops (index.html: the carousel's stages; site.css: .fl-*, .sv-*, .ac-*) are
// drawn at a fixed size and scaled to their frame: 960 px wide, and on phones 632 px (the
// floating control's right part) or 500 px. The loops are CSS keyframes, so the pauses
// above and the carousel's (only the shown slide runs) cover them.
const narrow = matchMedia("(max-width: 600px)");
for (const [sel, scene, phoneW] of [["[data-float]", ".fl-scene", 632], ["[data-files]", ".sv-scene", 500], ["[data-accounts]", ".ac-scene", 500]]) {
  const stage = document.querySelector(sel);
  if (!stage) continue;
  const el = stage.querySelector(scene);
  const fit = () => el.style.setProperty("--s", (stage.clientWidth / (narrow.matches ? phoneW : 960)).toFixed(4));
  new ResizeObserver(fit).observe(stage);
  fit();
}

// ---------- features carousel (index.html: [data-carousel]) ----------
// Slides in a horizontal scroll-snap track, with tabs that name them, arrows, swipe,
// and ←/→. Only the shown slide is reachable and animates; the others are inert and show
// their still, so a slide starts its loop from the beginning when it is shown. A link to
// a slide (#files) or to anything in it shows that slide.
const carousel = document.querySelector("[data-carousel]");
if (carousel) setupCarousel(carousel);

function setupCarousel(car) {
  const track = car.querySelector(".feat-track");
  const slides = [...track.querySelectorAll(":scope > .feat-slide")];
  const tabs = [...car.querySelectorAll('[role="tab"]')];
  const [prev, next] = car.querySelectorAll("[data-step]");
  let current = -1;

  function show(i) {
    if (i === current) return;
    current = i;
    tabs.forEach((t, k) => {
      t.setAttribute("aria-selected", String(k === i));
      t.tabIndex = k === i ? 0 : -1;
    });
    slides.forEach((s, k) => {
      s.inert = k !== i;
      s.classList.toggle("feat-idle", k !== i);
    });
    prev.disabled = i === 0;
    next.disabled = i === slides.length - 1;
    // On a phone the tab row scrolls on its own: bring the shown tab into it.
    const list = tabs[i].parentElement;
    if (list.scrollWidth > list.clientWidth) {
      const to = tabs[i].offsetLeft - (list.clientWidth - tabs[i].offsetWidth) / 2;
      list.scrollTo({ left: to, behavior: reduceMotion.matches ? "auto" : "smooth" });
    }
  }
  // Where slide i starts in the track (each slide is as wide as the track).
  const left = (i) => i * track.clientWidth;
  function go(i, smooth = true) {
    i = Math.max(0, Math.min(slides.length - 1, i));
    show(i);
    track.scrollTo({ left: left(i), behavior: smooth && !reduceMotion.matches ? "smooth" : "auto" });
  }

  // A swipe, a trackpad or a scrollbar: the slide it settles on is shown.
  let settle = 0;
  track.addEventListener(
    "scroll",
    () => {
      clearTimeout(settle);
      settle = setTimeout(() => show(Math.round(track.scrollLeft / track.clientWidth)), 120);
    },
    { passive: true },
  );
  // Keep the shown slide in place when the width changes.
  new ResizeObserver(() => current >= 0 && track.scrollTo({ left: left(current), behavior: "auto" })).observe(track);

  tabs.forEach((tab, i) => {
    tab.addEventListener("click", (e) => {
      e.preventDefault();
      go(i);
    });
    tab.addEventListener("keydown", (e) => {
      const keys = { Home: 0, End: tabs.length - 1, ArrowRight: i + 1, ArrowLeft: i - 1 };
      const to = keys[e.key];
      if (to === undefined || to < 0 || to >= tabs.length) return;
      e.preventDefault();
      tabs[to].focus();
      go(to);
    });
  });
  prev.addEventListener("click", () => go(current - 1));
  next.addEventListener("click", () => go(current + 1));
  // ←/→ anywhere else in the carousel (the arrows, the track, a slide's links).
  car.addEventListener("keydown", (e) => {
    if (e.defaultPrevented || e.altKey || e.metaKey || e.ctrlKey || (e.key !== "ArrowLeft" && e.key !== "ArrowRight")) return;
    if (e.target instanceof Element && e.target.closest('[role="tab"], input, textarea')) return;
    e.preventDefault();
    go(current + (e.key === "ArrowRight" ? 1 : -1));
  });

  // #files, #away, #accounts, or anything inside a slide (a heading's id).
  function fromHash() {
    let target = null;
    try {
      if (location.hash.length > 1) target = document.getElementById(decodeURIComponent(location.hash.slice(1)));
    } catch {}
    const i = target ? slides.findIndex((s) => s.contains(target)) : -1;
    if (i < 0) return false;
    go(i, false);
    car.scrollIntoView({ block: "start" });
    return true;
  }
  addEventListener("hashchange", fromHash);
  if (!fromHash()) go(0, false);
  // The browser scrolls to a hash target once the page has loaded: land on the carousel.
  addEventListener("load", () => location.hash && fromHash(), { once: true });
}

// Sections and cards below the fold fade and rise in as they arrive. Nothing already
// on screen is hidden, and with reduced motion nothing is hidden at all.
const REVEAL = ".features .wrap, .pit .pit-panel, .closing .wrap, .dl, .lane, .release, .board-wrap, .table-scroll, .callout, .keys";
if (!reduceMotion.matches && "IntersectionObserver" in window) {
  const io = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        io.unobserve(e.target);
        e.target.classList.add("is-in");
      }
    },
    { rootMargin: "0px 0px -6% 0px" },
  );
  for (const el of document.querySelectorAll(REVEAL)) {
    if (el.getBoundingClientRect().top < window.innerHeight) continue;
    // Cards in a row arrive one after another.
    const row = [...el.parentElement.children].filter((c) => c.matches(REVEAL));
    el.style.setProperty("--i", String(row.indexOf(el) % 3));
    el.classList.add("reveal");
    el.addEventListener(
      "transitionend",
      (ev) => {
        if (ev.target !== el || ev.propertyName !== "transform") return;
        el.classList.remove("reveal", "is-in");
        el.style.removeProperty("--i");
      },
    );
    io.observe(el);
  }
}

// Headlight sweep (site.css: .cta.sweep): once as the hero settles after the intro, and on
// touch screens, which have no hover, once as each other call to action comes into view.
function sweep(cta, delay) {
  setTimeout(() => {
    cta.classList.add("sweep");
    setTimeout(() => cta.classList.remove("sweep"), 1100);
  }, delay);
}
if (!reduceMotion.matches) {
  const heroCta = document.querySelector(".hero .cta");
  if (heroCta && !root.classList.contains("intro-seen")) sweep(heroCta, 1900);
  if (matchMedia("(hover: none)").matches && "IntersectionObserver" in window) {
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue;
          io.unobserve(e.target);
          sweep(e.target, 150);
        }
      },
      { threshold: 0.6 },
    );
    for (const el of document.querySelectorAll(".cta")) if (!el.closest(".hero")) io.observe(el);
  }
}

// Reading progress: a car on the header bar (site.css: .pbar), on <body data-progress> pages.
if (document.body.hasAttribute("data-progress") && !reduceMotion.matches) setupProgress();

function setupProgress() {
  const head = document.querySelector(".site-head .wrap");
  if (!head) return;
  const bar = document.createElement("div");
  bar.className = "pbar";
  bar.setAttribute("aria-hidden", "true");
  bar.innerHTML =
    '<span class="pb-lane"></span><span class="pb-fill"></span><span class="pb-pits"></span>' +
    '<span class="pb-track"><span class="pb-rider"><svg class="pb-car" viewBox="0 0 22 16">' +
    '<path class="s" d="M1 5.5h3M0 8h4M1 10.5h3" fill="none" stroke-width="1.2" stroke-linecap="round"/>' +
    '<path class="w" d="M5.8 2h1.9q.8 0 .8.8v1.4q0 .8-.8.8H5.8q-.8 0-.8-.8V2.8q0-.8.8-.8zm0 9h1.9q.8 0 .8.8v1.4q0 .8-.8.8H5.8q-.8 0-.8-.8v-1.4q0-.8.8-.8zm10-9h1.9q.8 0 .8.8v1.4q0 .8-.8.8h-1.9q-.8 0-.8-.8V2.8q0-.8.8-.8zm0 9h1.9q.8 0 .8.8v1.4q0 .8-.8.8h-1.9q-.8 0-.8-.8v-1.4q0-.8.8-.8z"/>' +
    '<path class="b" d="M6 6.2Q6 4.6 8 4.6H17Q21 4.6 21.5 8 21 11.4 17 11.4H8Q6 11.4 6 9.8Z"/>' +
    '<rect class="c" x="12" y="6.6" width="3.4" height="2.8" rx="1.2"/></svg></span></span>' +
    '<span class="pb-flag"></span>';
  head.append(bar);
  const pits = bar.querySelector(".pb-pits");

  // Measured on load and on resize only; the scroll path below never reads layout.
  let max = 1;
  function measure() {
    max = Math.max(1, document.documentElement.scrollHeight - innerHeight);
    const starts = [...document.querySelectorAll("main > section:not(.hero), main.page h2")]
      .map((el) => (el.getBoundingClientRect().top + scrollY - 88) / max)
      .filter((p) => p > 0.02 && p < 0.98)
      .slice(0, 12);
    pits.replaceChildren(
      ...starts.map((p) => {
        const m = document.createElement("i");
        m.className = "pb-pit";
        m.style.left = `${(p * 100).toFixed(2)}%`;
        return m;
      }),
    );
    paint();
  }

  // Without scroll timelines, a rAF-throttled handler sets --p (one style write per frame).
  const timeline = CSS.supports("animation-timeline: scroll()");
  let queued = false;
  function paint() {
    queued = false;
    if (!timeline) bar.style.setProperty("--p", Math.min(1, Math.max(0, scrollY / max)).toFixed(4));
  }
  if (!timeline)
    addEventListener("scroll", () => {
      if (!queued) {
        queued = true;
        requestAnimationFrame(paint);
      }
    }, { passive: true });
  let resizeQueued = false;
  const remeasure = () => {
    if (resizeQueued) return;
    resizeQueued = true;
    requestAnimationFrame(() => {
      resizeQueued = false;
      measure();
    });
  };
  new ResizeObserver(remeasure).observe(document.body);
  addEventListener("resize", remeasure, { passive: true });
}

function currentTheme() {
  const set = root.dataset.theme;
  if (set === "light" || set === "dark") return set;
  return matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
}

const themeListeners = [];

function syncThemeButton(btn) {
  const next = currentTheme() === "dark" ? "light" : "dark";
  btn.setAttribute("aria-label", `Switch to ${next} theme`);
  btn.title = `Switch to ${next} theme`;
}

for (const btn of document.querySelectorAll("[data-theme-toggle]")) {
  syncThemeButton(btn);
  btn.addEventListener("click", () => {
    const next = currentTheme() === "dark" ? "light" : "dark";
    root.dataset.theme = next;
    try {
      localStorage.setItem("pitwall-theme", next);
    } catch {
      /* private mode: theme just isn't remembered */
    }
    syncThemeButton(btn);
    themeListeners.forEach((f) => f());
  });
}

// ---------- demo: the live app (the app's browser mock), its views on tabs ----------
const demo = document.querySelector("[data-demo]");
if (demo) setupDemo(demo);

function setupDemo(demo) {
  const tabs = [...demo.querySelectorAll('[role="tab"]')];
  const panel = demo.querySelector('[role="tabpanel"]');
  const copies = [...demo.querySelectorAll("[data-copy]")];
  const body = demo.querySelector("[data-frame]");
  const skeleton = body.querySelector(".skeleton");
  const missing = body.querySelector("[data-demo-missing]");
  // The site can live under a sub-path (GitHub Pages: /<repo>/); Vite sets BASE_URL.
  const DEMO_DIR = `${import.meta.env.BASE_URL}demo/`;
  let iframe = null;
  let ready = false;

  const selected = () => tabs.find((t) => t.getAttribute("aria-selected") === "true") ?? tabs[0];
  // Messages for the app's demo bridge (src/lib/demoBridge.ts), same origin only.
  const send = (msg) => iframe?.contentWindow?.postMessage({ type: "pitwall-demo", ...msg }, location.origin);

  function select(tab) {
    for (const t of tabs) {
      const on = t === tab;
      t.setAttribute("aria-selected", String(on));
      t.tabIndex = on ? 0 : -1;
    }
    panel.setAttribute("aria-labelledby", tab.id);
    for (const c of copies) c.hidden = c.dataset.copy !== tab.dataset.view;
    // On a phone the tab row scrolls on its own: bring the shown tab into it.
    const list = tab.parentElement;
    if (list.scrollWidth > list.clientWidth) {
      const to = tab.offsetLeft - (list.clientWidth - tab.offsetWidth) / 2;
      list.scrollTo({ left: to, behavior: reduceMotion.matches ? "auto" : "smooth" });
    }
    fit();
    if (!ready) return; // sent once the app says it's ready
    const keep = document.activeElement;
    send({ view: tab.dataset.view });
    // The app focuses its terminal when an agent is selected; keep keyboard focus on the tab.
    setTimeout(() => {
      if (document.activeElement === iframe && keep instanceof HTMLElement) keep.focus({ preventScroll: true });
    }, 400);
  }

  tabs.forEach((tab, i) => {
    tab.addEventListener("click", () => select(tab));
    tab.addEventListener("keydown", (e) => {
      const d = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
      const to = e.key === "Home" ? tabs[0] : e.key === "End" ? tabs[tabs.length - 1] : d ? tabs[(i + d + tabs.length) % tabs.length] : null;
      if (!to) return;
      e.preventDefault();
      to.focus();
      select(to);
    });
  });

  // The app lays out by window width and only shows its full sidebar from 1100 px, so it
  // renders at 1120x700 and is scaled to the frame. Phones (frame 4:5) show its left half,
  // or its right half for views whose tab has data-phone="right" (a files or web tile
  // beside the terminal, the Next up popover on its tile).
  const APP_W = 1120;
  const APP_H = 700;
  const narrow = matchMedia("(max-width: 600px)");
  function fit() {
    if (!iframe) return;
    const scale = body.clientWidth / (narrow.matches ? APP_W / 2 : APP_W);
    const shift = narrow.matches && selected().dataset.phone === "right" ? -APP_W / 2 : 0;
    Object.assign(iframe.style, { width: `${APP_W}px`, height: `${APP_H}px`, transform: `scale(${scale}) translateX(${shift}px)` });
  }
  new ResizeObserver(fit).observe(body);
  narrow.addEventListener("change", fit);

  // The overview cards below the demo (index.html: .layout-ov) show their view here.
  for (const link of document.querySelectorAll("[data-demo-view]")) {
    const tab = tabs.find((t) => t.dataset.view === link.dataset.demoView);
    if (!tab) continue;
    link.addEventListener("click", (e) => {
      e.preventDefault();
      select(tab);
      demo.scrollIntoView({ behavior: reduceMotion.matches ? "auto" : "smooth", block: "start" });
      tab.focus({ preventScroll: true });
    });
  }

  // The app follows the page's theme toggle, not just the OS.
  const syncTheme = () => send({ theme: currentTheme() });
  themeListeners.push(syncTheme);
  matchMedia("(prefers-color-scheme: light)").addEventListener("change", syncTheme);

  function start() {
    fetch(`${DEMO_DIR}index.html`)
      .then((r) => (r.ok ? r.text() : ""))
      .then((html) => {
        if (!html.includes('id="root"')) throw new Error("no demo");
        // A fresh demo every visit: the mock keeps its state in localStorage under "pitwall.".
        try {
          for (const k of Object.keys(localStorage)) if (k.startsWith("pitwall.")) localStorage.removeItem(k);
        } catch {
          /* storage blocked: the demo starts from its saved state */
        }
        iframe = document.createElement("iframe");
        iframe.title = "Pitwall, running live with made-up projects";
        iframe.src = `${DEMO_DIR}?onboarded&shots=1&demo=1&theme=${currentTheme()}`;
        iframe.style.visibility = "hidden";
        body.append(iframe);
        fit();
        // If the ready message never comes (slow device, bridge changed), show it anyway.
        setTimeout(reveal, 12000);
      })
      .catch(() => (missing.hidden = false));
  }

  function reveal() {
    if (!iframe || !iframe.style.visibility) return;
    iframe.style.visibility = "";
    skeleton.hidden = true;
  }

  window.addEventListener("message", (e) => {
    if (e.origin !== location.origin || !iframe || e.source !== iframe.contentWindow) return;
    if (e.data?.type !== "pitwall-demo-ready") return;
    ready = true;
    syncTheme();
    select(selected());
    // Give the view a moment to settle before the placeholder goes.
    setTimeout(reveal, 250);
  });

  // Load lazily: after the page itself has loaded, and once the frame is near the viewport,
  // so the hero never waits on the app.
  const near = () => {
    if (!("IntersectionObserver" in window)) return start();
    const io = new IntersectionObserver(
      (entries) => {
        if (!entries.some((en) => en.isIntersecting)) return;
        io.disconnect();
        start();
      },
      { rootMargin: "300px 0px" },
    );
    io.observe(body);
  };
  if (document.readyState === "complete") near();
  else addEventListener("load", near, { once: true });
}

// ---------- downloads: the visitor's OS first, links from the latest release ----------
// Which desktop OS this is, or null (phones, tablets, unknown). userAgentData where the
// browser has it, the user agent string otherwise. iPads that report "Macintosh" have touch.
function detectOS() {
  const p = (navigator.userAgentData?.platform || "").toLowerCase();
  const ua = navigator.userAgent || "";
  if (p) {
    if (p === "macos") return "mac";
    if (p === "windows") return "win";
    if (p === "linux") return "linux";
    return null; // Android, iOS, Chrome OS: no build
  }
  if (/Android|iPhone|iPad|iPod/i.test(ua)) return null;
  if (/Macintosh|Mac OS X/i.test(ua)) return navigator.maxTouchPoints > 1 ? null : "mac";
  if (/Windows/i.test(ua)) return "win";
  if (/Linux|X11/i.test(ua) && !/CrOS/i.test(ua)) return "linux";
  return null;
}

const OS_NAMES = { mac: "macOS", win: "Windows", linux: "Linux" };
const os = detectOS();

// "Download" buttons elsewhere on the site name the visitor's OS.
if (os) for (const el of document.querySelectorAll("[data-dl-label]")) el.textContent = `Download for ${OS_NAMES[os]}`;

const downloads = document.querySelector("[data-downloads]");
if (downloads) setupDownloads(downloads);

function setupDownloads(page) {
  const grid = page.querySelector("[data-dl-grid]");
  const card = os && grid.querySelector(`[data-os="${os}"]`);
  if (card) {
    grid.prepend(card);
    card.classList.add("is-you");
    card.querySelector(".dl-you").hidden = false;
  }

  const status = page.querySelector("[data-release-status]");
  const fail = (message) => {
    status.textContent = message;
    status.hidden = false;
  };

  const CACHE = "pitwall-release";
  let cached = null;
  try {
    const c = JSON.parse(sessionStorage.getItem(CACHE) || "null");
    if (c && Date.now() - c.at < 10 * 60 * 1000) cached = c.release;
  } catch {
    /* storage blocked: fetch every time */
  }

  const got = cached
    ? Promise.resolve(cached)
    : fetch(page.dataset.api, { headers: { Accept: "application/vnd.github+json" } }).then((r) => {
        if (r.status === 404) throw new Error("none");
        if (!r.ok) throw new Error(String(r.status));
        return r.json();
      });

  got
    .then((release) => {
      const assets = Array.isArray(release?.assets) ? release.assets : [];
      if (!release?.tag_name || !assets.length) throw new Error("none");
      try {
        sessionStorage.setItem(CACHE, JSON.stringify({ at: Date.now(), release }));
      } catch {
        /* not cached */
      }
      fill(release, assets);
    })
    .catch((e) => {
      const why = e.message === "none" ? "No published release found." : "Couldn't reach GitHub for the latest version.";
      fail(`${why} The buttons open the Releases page on GitHub, which lists every file.`);
    });

  function fill(release, assets) {
    const version = release.tag_name.replace(/^v/, "");
    const date = release.published_at ? new Date(release.published_at) : null;
    const when = date && !isNaN(date) ? date.toLocaleDateString("en", { day: "numeric", month: "short", year: "numeric" }) : "";
    page.querySelector("[data-release-label]").textContent = `Download · v${version}`;
    const info = page.querySelector("[data-release-info]");
    info.textContent = "";
    const strong = document.createElement("strong");
    strong.textContent = `v${version}`;
    info.append(strong, when ? ` · released ${when}` : "");
    const changelog = page.querySelector("[data-changelog]");
    changelog.hash = `v${version.toLowerCase().replace(/[^a-z0-9]+/g, "-")}`;

    // A platform with none of its files in the release (Windows, until its
    // port is verified) shows "coming soon" instead of dead buttons.
    for (const c of grid.querySelectorAll("[data-os]")) {
      const suffixes = [...c.querySelectorAll("a[data-asset]")].map((a) => a.dataset.asset);
      if (suffixes.length && !suffixes.some((s) => assets.some((x) => typeof x?.name === "string" && x.name.endsWith(s)))) {
        c.classList.add("is-soon");
        const soon = c.querySelector("[data-soon]");
        if (soon) soon.hidden = false;
      }
    }

    let missing = 0;
    for (const a of page.querySelectorAll("[data-os]:not(.is-soon) a[data-asset]")) {
      const suffix = a.dataset.asset;
      const asset = assets.find((x) => typeof x?.name === "string" && x.name.endsWith(suffix) && /^https:\/\//.test(x.browser_download_url || ""));
      const row = a.closest("li");
      if (!asset) {
        if (row) missing++;
        continue;
      }
      a.href = asset.browser_download_url;
      if (!row) continue;
      row.querySelector("[data-name]").textContent = asset.name;
      if (asset.size) row.querySelector("[data-size]").textContent = `· ${formatSize(asset.size)}`;
    }
    if (missing) fail(`Some files aren't in v${version} yet; their buttons open the Releases page.`);
  }
}

function formatSize(bytes) {
  return bytes >= 1024 * 1024 ? `${(bytes / 1024 / 1024).toFixed(1)} MB` : `${Math.max(1, Math.round(bytes / 1024))} KB`;
}
