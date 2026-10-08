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
if (motionIO) for (const el of document.querySelectorAll("[data-demo], .board-wrap, .lanes")) motionIO.observe(el);

// Sections and cards below the fold fade and rise in as they arrive. Nothing already
// on screen is hidden, and with reduced motion nothing is hidden at all.
const REVEAL = ".pit-panel, .closing .wrap, .dl, .lane, .release, .board-wrap, .table-scroll, .callout, .keys";
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

// ---------- demo: tabs drive the video, or the live app once it's loaded ----------
const demo = document.querySelector("[data-demo]");
if (demo) setupDemo(demo);

function setupDemo(demo) {
  const tabs = [...demo.querySelectorAll('[role="tab"]')];
  const panel = demo.querySelector('[role="tabpanel"]');
  const copies = [...demo.querySelectorAll("[data-copy]")];
  const video = demo.querySelector("[data-video]");
  const body = demo.querySelector(".frame-body");
  const skeleton = demo.querySelector(".skeleton");
  const liveBtn = demo.querySelector("[data-live]");
  const playBtn = demo.querySelector("[data-play]");
  // The site can live under a sub-path (GitHub Pages: /<repo>/); Vite sets BASE_URL.
  const DEMO_DIR = `${import.meta.env.BASE_URL}demo/`;
  const DEMO_URL = `${DEMO_DIR}?onboarded&shots=1&demo=1`;
  let iframe = null;
  let live = false;
  // Once a visitor picks a tab, the video loops that chapter and the highlight
  // stays put; before that it plays the whole tour and the tabs follow along.
  let pinned = null;

  // Chapters in the recording, in time order, for "which tab is playing".
  const chapters = tabs.map((t) => ({ tab: t, t: Number(t.dataset.t) })).sort((a, b) => a.t - b.t);
  const chapterEnd = (c) => {
    const i = chapters.indexOf(c);
    return i + 1 < chapters.length ? chapters[i + 1].t : Infinity;
  };

  // Chapter fill: a thin bar under the playing tab shows how far its chapter has got.
  let fillTab = null;
  let fillRaf = 0;
  let demoOnScreen = true;
  function chapterAt(t) {
    let cur = chapters[0];
    for (const c of chapters) if (t + 0.05 >= c.t) cur = c;
    return cur;
  }
  function paintFill() {
    fillRaf = 0;
    if (live) {
      fillTab?.style.removeProperty("--p");
      fillTab = null;
      return;
    }
    const cur = pinned ?? chapterAt(video.currentTime);
    const end = Math.min(chapterEnd(cur), video.duration || Infinity);
    const p = Number.isFinite(end) && end > cur.t ? Math.min(1, Math.max(0, (video.currentTime - cur.t) / (end - cur.t))) : 0;
    if (fillTab && fillTab !== cur.tab) fillTab.style.removeProperty("--p");
    cur.tab.style.setProperty("--p", p.toFixed(4));
    fillTab = cur.tab;
    if (!video.paused && demoOnScreen && !document.hidden && !reduceMotion.matches) fillRaf = requestAnimationFrame(paintFill);
  }
  function queueFill() {
    if (!fillRaf) fillRaf = requestAnimationFrame(paintFill);
  }
  for (const ev of ["play", "seeked", "timeupdate", "loadedmetadata"]) video.addEventListener(ev, queueFill);
  if ("IntersectionObserver" in window)
    new IntersectionObserver(([e]) => {
      demoOnScreen = e.isIntersecting;
      if (demoOnScreen) queueFill();
    }).observe(demo);
  document.addEventListener("visibilitychange", queueFill);

  function select(tab, { fromVideo = false } = {}) {
    for (const t of tabs) {
      const on = t === tab;
      t.setAttribute("aria-selected", String(on));
      t.tabIndex = on ? 0 : -1;
    }
    panel.setAttribute("aria-labelledby", tab.id);
    for (const c of copies) c.hidden = c.dataset.copy !== tab.dataset.view;
    if (fromVideo) return;
    if (live && iframe) {
      const keep = document.activeElement;
      iframe.contentWindow?.postMessage({ type: "pitwall-demo", view: tab.dataset.view }, location.origin);
      // The app focuses its terminal when an agent is selected; keep keyboard focus on the tab.
      setTimeout(() => {
        if (document.activeElement === iframe && keep instanceof HTMLElement) keep.focus({ preventScroll: true });
      }, 400);
    } else {
      pinned = chapters.find((c) => c.tab === tab) ?? null;
      video.currentTime = Number(tab.dataset.t);
      if (!reduceMotion.matches) video.play().catch(() => {});
    }
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

  // Video: pick the file for the page theme (the toggle can differ from the OS setting).
  function setVideoTheme() {
    const t = currentTheme();
    const src = video.dataset[t];
    video.poster = video.dataset[t === "dark" ? "posterDark" : "posterLight"];
    if (video.currentSrc && video.currentSrc.endsWith(src)) return;
    const at = video.currentTime;
    const playing = !video.paused;
    video.src = src;
    video.addEventListener("loadedmetadata", () => {
      video.currentTime = at;
      if (playing || !reduceMotion.matches) video.play().catch(() => {});
    }, { once: true });
  }
  setVideoTheme();
  themeListeners.push(setVideoTheme);
  matchMedia("(prefers-color-scheme: light)").addEventListener("change", setVideoTheme);

  // Follow the recording with the tab strip while it plays.
  video.addEventListener("timeupdate", () => {
    if (live || video.seeking) return;
    if (pinned) {
      // Loop the picked chapter (the last one runs to the end of the file, then wraps).
      const t = video.currentTime;
      if (t + 0.05 >= chapterEnd(pinned) || t + 0.05 < pinned.t) video.currentTime = pinned.t;
      return;
    }
    let cur = chapters[0];
    for (const c of chapters) if (video.currentTime + 0.05 >= c.t) cur = c;
    if (cur.tab.getAttribute("aria-selected") !== "true") select(cur.tab, { fromVideo: true });
  });

  // Reduced motion: no autoplay; a Play/Pause button instead.
  function syncPlay() {
    playBtn.textContent = video.paused ? "Play" : "Pause";
    playBtn.setAttribute("aria-label", video.paused ? "Play video" : "Pause video");
  }
  playBtn.hidden = false;
  playBtn.addEventListener("click", () => (video.paused ? video.play() : video.pause()));
  video.addEventListener("play", syncPlay);
  video.addEventListener("pause", syncPlay);
  if (reduceMotion.matches) video.pause();
  else video.play().catch(() => {});
  syncPlay();

  // Live app: only offered on wide screens, and only if the demo build is deployed.
  const wide = matchMedia("(min-width: 900px)");
  fetch(`${DEMO_DIR}index.html`, { method: "GET" })
    .then((r) => (r.ok ? r.text() : ""))
    .then((html) => {
      if (!html.includes('id="root"')) return;
      liveBtn.hidden = !wide.matches;
      wide.addEventListener("change", () => (liveBtn.hidden = !wide.matches && !live));
    })
    .catch(() => {});

  liveBtn.addEventListener("click", () => (live ? stopLive() : startLive()));

  function startLive() {
    // A fresh demo every time: the mock keeps its state in localStorage under "pitwall.".
    try {
      for (const k of Object.keys(localStorage)) if (k.startsWith("pitwall.")) localStorage.removeItem(k);
    } catch {
      /* storage blocked: the demo just starts from its saved state */
    }
    live = true;
    video.pause();
    queueFill();
    video.hidden = true;
    playBtn.hidden = true;
    skeleton.hidden = false;
    iframe = document.createElement("iframe");
    iframe.title = "Pitwall, running live with demo projects";
    iframe.src = DEMO_URL;
    iframe.style.visibility = "hidden";
    body.append(iframe);
    fitIframe();
    // If the ready message never comes (slow device, bridge changed), show it anyway.
    revealTimer = setTimeout(reveal, 10000);
    liveBtn.textContent = "Back to video";
  }

  // The app lays out by window width and only shows its full sidebar from 1100 px,
  // so it renders at the size the video was recorded at and is scaled to the frame.
  const APP_W = 1120;
  const APP_H = 700;
  let revealTimer = 0;
  function fitIframe() {
    if (!iframe) return;
    const scale = body.clientWidth / APP_W;
    Object.assign(iframe.style, {
      width: `${APP_W}px`,
      height: `${APP_H}px`,
      transformOrigin: "0 0",
      transform: `scale(${scale})`,
    });
  }
  new ResizeObserver(fitIframe).observe(body);

  function reveal() {
    clearTimeout(revealTimer);
    if (!iframe) return;
    skeleton.hidden = true;
    iframe.style.visibility = "";
  }

  function stopLive() {
    clearTimeout(revealTimer);
    live = false;
    queueFill();
    iframe?.remove();
    iframe = null;
    skeleton.hidden = true;
    video.hidden = false;
    playBtn.hidden = false;
    liveBtn.textContent = "Try it live";
    if (!reduceMotion.matches) video.play().catch(() => {});
    liveBtn.focus();
  }

  window.addEventListener("message", (e) => {
    if (e.origin !== location.origin || !iframe || e.source !== iframe.contentWindow) return;
    if (e.data?.type !== "pitwall-demo-ready") return;
    reveal();
    const tab = tabs.find((t) => t.getAttribute("aria-selected") === "true");
    if (tab) select(tab);
  });
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
