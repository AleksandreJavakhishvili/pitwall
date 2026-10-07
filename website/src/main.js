import "./site.css";

const root = document.documentElement;
const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)");

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
