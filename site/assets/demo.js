// Interactive demo: a mock of the desktop app (tuner-gtk) and the terminal UI
// (tuner) running on fake data, sharing one state model like the real
// front-ends share tuner_app. Behaviour follows docs/keybindings.md, docs/gui.md
// and docs/stream-limits.md; keep them in sync (see docs/website.md).
(() => {
  const D = window.TunerData;
  const Theme = window.TunerTheme;

  // ---------------------------------------------------------------- helpers
  const h = (tag, attrs = {}, ...kids) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (v == null || v === false) continue;
      if (k === "class") e.className = v;
      else if (k === "text") e.textContent = v;
      else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
      else e.setAttribute(k, v === true ? "" : v);
    }
    for (const kid of kids.flat()) {
      if (kid != null && kid !== false) e.append(kid);
    }
    return e;
  };

  const ICONS = {
    search: '<path d="M10.5 3a7.5 7.5 0 0 1 5.9 12.1l4.3 4.3-1.4 1.4-4.3-4.3A7.5 7.5 0 1 1 10.5 3zm0 2a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11z"/>',
    play: '<path d="M7 4.5v15l13-7.5z"/>',
    pause: '<path d="M6 4h4v16H6zM14 4h4v16h-4z"/>',
    stop: '<path d="M6 6h12v12H6z"/>',
    info: '<path d="M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zm0 2a8 8 0 1 1 0 16 8 8 0 0 1 0-16zm-1 4v2h2V8zm0 4v6h2v-6z"/>',
    list: '<path d="M4 6h16v2H4zm0 5h16v2H4zm0 5h16v2H4z"/>',
    grid: '<path d="M4 4h7v7H4zm9 0h7v7h-7zM4 13h7v7H4zm9 0h7v7h-7z"/>',
    refresh: '<path d="M12 4a8 8 0 0 1 7.5 5.2h-2.2A6 6 0 1 0 18 14h2.1A8 8 0 1 1 12 4zm5 0h3v5h-5z"/>',
    palette: '<path d="M12 2a10 10 0 0 0 0 20c1.7 0 2.5-1.2 2.2-2.4-.3-1.2.4-2.1 1.6-2.1H18a4 4 0 0 0 4-4C22 6.5 17.5 2 12 2zm-5 10a1.5 1.5 0 1 1 0-3 1.5 1.5 0 0 1 0 3zm3-4a1.5 1.5 0 1 1 0-3 1.5 1.5 0 0 1 0 3zm5 0a1.5 1.5 0 1 1 0-3 1.5 1.5 0 0 1 0 3z"/>',
    user: '<path d="M12 12a5 5 0 1 0 0-10 5 5 0 0 0 0 10zm0 2c-4.4 0-8 2-8 5v3h16v-3c0-3-3.6-5-8-5z"/>',
    chevron: '<path d="M6 9l6 6 6-6" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/>',
    back: '<path d="M15 5l-7 7 7 7" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/>',
    close: '<path d="M6 6l12 12M18 6L6 18" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"/>',
    check: '<path d="M5 12.5l4.5 4.5L19 7.5" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/>',
    // embedded video's control bar
    rewind: '<path d="M11.5 6v12L3 12zm9 0v12L12 12z"/>',
    forward: '<path d="M12.5 6v12L21 12zm-9 0v12L12 12z"/>',
    volume: '<path fill="currentColor" d="M3 9h4l5-4v14l-5-4H3z"/><path d="M15.5 8.5a5 5 0 0 1 0 7M18 6a8.5 8.5 0 0 1 0 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>',
    muted: '<path fill="currentColor" d="M3 9h4l5-4v14l-5-4H3z"/><path d="M16 9.5l5 5M21 9.5l-5 5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>',
    subtitles: '<rect x="3" y="5" width="18" height="14" rx="2.5" fill="none" stroke="currentColor" stroke-width="2"/><path d="M7 12h3.5M13 12h4M7 15.5h6M15.5 15.5h1.5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>',
    eye: '<path d="M2 12s3.6-6.5 10-6.5S22 12 22 12s-3.6 6.5-10 6.5S2 12 2 12z" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/><circle cx="12" cy="12" r="3" fill="currentColor"/>',
    stats: '<path d="M3 19h4v-5H3zm7 0h4V10h-4zm7 0h4V4h-4z"/>',
    detach: '<path d="M13.5 4H20v6.5M20 4l-8.5 8.5M17.5 14v6H4V6.5h6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>',
    cinema: '<rect x="2" y="6.5" width="20" height="11" rx="2" fill="none" stroke="currentColor" stroke-width="2"/>',
    full: '<path d="M4 9V4h5M15 4h5v5M20 15v5h-5M9 20H4v-5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>',
    unfull: '<path d="M9 4v5H4M20 9h-5V4M15 20v-5h5M4 15h5v5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>',
  };
  const icon = (name, cls = "") => {
    const s = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    s.setAttribute("viewBox", "0 0 24 24");
    s.setAttribute("aria-hidden", "true");
    s.setAttribute("class", `ico ${cls}`.trim());
    s.innerHTML = ICONS[name];
    if (!ICONS[name].includes("stroke")) s.setAttribute("fill", "currentColor");
    return s;
  };

  const initials = (name) =>
    name
      .replace(/\(.*?\)/g, "")
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((w) => w[0].toUpperCase())
      .join("");

  const logo = (item, cls = "") =>
    h("span", {
      class: `logo ${item.logo ? "logo-num" : ""} ${cls}`.trim(),
      style: `--h:${item.hue}`,
      text: item.logo || initials(item.show || item.name),
      "aria-hidden": "true",
    });

  // Same format as the GUI's time label: hours only when needed.
  const clock = (secs) => {
    const t = Math.max(0, Math.floor(secs));
    const [hh, m, ss] = [Math.floor(t / 3600), Math.floor(t / 60) % 60, t % 60];
    const pad = (n) => String(n).padStart(2, "0");
    return hh ? `${hh}:${pad(m)}:${pad(ss)}` : `${m}:${pad(ss)}`;
  };

  const touch = matchMedia("(pointer: coarse)");
  const plural = (n, w) => `${n} ${w}${n === 1 ? "" : "s"}`;
  const keepInView = (box, row) => {
    if (!box || !row) return;
    const b = box.getBoundingClientRect();
    const r = row.getBoundingClientRect();
    if (r.top < b.top) box.scrollTop -= b.top - r.top + 4;
    else if (r.bottom > b.bottom) box.scrollTop += r.bottom - b.bottom + 4;
  };

  // ------------------------------------------------------------------ model
  const s = {
    account: 0,
    section: "live",
    group: 0,
    query: "",
    sel: 0,
    series: null,
    seriesSel: 0,
    dialog: null,
    playing: null, // { item, paused, pos, edge, aid, sid }
    detached: false, // moved to mpv's own window ("Open in mpv window")
    volume: 100,
    muted: false,
    showUrl: false,
    showDetails: true,
    status: { text: "Ready. Press Enter on a channel to play.", kind: "" },
    quit: false,
    view: "gui",
  };

  const subs = [];
  const emit = () => subs.forEach((fn) => fn());
  const say = (text, kind = "") => (s.status = { text, kind });

  const acct = () => D.accounts[s.account];
  const baseItems = () => {
    if (s.series) return s.series.episodes;
    if (s.section === "movies") return D.movies;
    if (s.section === "series") return D.series;
    return acct().m3u ? D.sample : D.channels;
  };
  const allLabel = () =>
    s.series ? "All episodes" : { live: "All channels", movies: "All movies", series: "All series" }[s.section];
  const groups = () => {
    const base = baseItems();
    const names = [];
    base.forEach((i) => names.includes(i.group) || names.push(i.group));
    return [
      { name: null, label: allLabel(), count: base.length },
      ...names.map((n) => ({ name: n, label: n, count: base.filter((i) => i.group === n).length })),
    ];
  };
  const items = () => {
    const g = groups()[s.group];
    const q = s.query.trim().toLowerCase();
    return baseItems().filter(
      (i) => (!g || !g.name || i.group === g.name) && (!q || i.name.toLowerCase().includes(q)),
    );
  };
  const clamp = () => {
    s.group = Math.max(0, Math.min(s.group, groups().length - 1));
    s.sel = Math.max(0, Math.min(s.sel, items().length - 1));
  };
  const selected = () => items()[s.sel] || null;
  const connections = () => acct().active + (s.playing ? 1 : 0);
  const sectionName = () =>
    s.series ? "Episodes" : { live: "Channels", movies: "Movies", series: "Series" }[s.section];
  const busy = () => !!(s.dialog || s.quit);

  const summary = () => {
    const a = acct();
    const n = plural(a.m3u ? D.sample.length : D.channels.length, "live channel");
    const parts = [`${a.name}: ${n}`];
    if (a.expires) parts.push(`expires ${a.expires}`);
    if (!a.m3u) parts.push(`${connections()}/${a.limit} connections`);
    parts.push(`limit ${plural(a.limit, "stream")}`);
    return parts.join(" · ");
  };

  // What mpv would report for a stream: length, video parameters, tracks.
  const isAudio = (item) => item.format === "AAC";
  const duration = (item) =>
    item.kind === "movie" ? (82 + (item.hue % 47)) * 60 + 17 : item.kind === "episode" ? (38 + (item.hue % 11)) * 60 + 41 : null;
  const params = (item) => {
    if (isAudio(item)) return null;
    const hd = item.kind !== "live" || / HD$/.test(item.name);
    return {
      w: hd ? 1920 : 1280,
      h: hd ? 1080 : 720,
      fps: item.kind !== "live" ? 23.976 : item.group === "Sports" ? 50 : 25,
      mbps: (hd ? 3.4 : 2.1) + (item.hue % 17) / 10,
    };
  };
  const LANGS = { eng: "English", tur: "Turkish", deu: "German", spa: "Spanish", fra: "French" };
  const tracks = (item) => {
    const t = (id, lang, extra) => ({ id, lang, title: "", codec: "AAC", ch: "stereo", def: false, forced: false, ...extra });
    if (item.kind === "movie")
      return {
        audio: [t(1, "eng", { codec: "E-AC-3", ch: "5.1", def: true }), t(2, "tur", { title: "Dubbed" }), t(3, "deu")],
        subs: [t(1, "eng", { codec: "SubRip" }), t(2, "eng", { title: "SDH", codec: "SubRip" }), t(3, "tur", { codec: "SubRip" }), t(4, "spa", { codec: "SubRip", forced: true })],
      };
    if (item.kind === "episode")
      return { audio: [t(1, "eng", { def: true })], subs: [t(1, "eng", { codec: "WebVTT" }), t(2, "tur", { codec: "WebVTT" })] };
    if (item.name.startsWith("Café")) return { audio: [t(1, "tur", { def: true }), t(2, "eng")], subs: [] };
    if (item.group === "News") return { audio: [t(1, "eng", { def: true })], subs: [t(1, "eng", { title: "Teletext", codec: "DVB" })] };
    return { audio: [t(1, "eng", { def: true })], subs: [] };
  };

  const act = {
    setQuery(q) {
      s.query = q;
      s.sel = 0;
      emit();
    },
    setGroup(i) {
      s.group = i;
      s.sel = 0;
      clamp();
      emit();
    },
    setSel(i) {
      s.sel = i;
      clamp();
      emit();
    },
    move(d) {
      act.setSel(s.sel + d);
    },
    setSection(sec) {
      if (s.quit) return;
      if (sec !== "live" && !acct().vod) {
        say("Movies and series need an Xtream login. Switch account (a).", "warn");
        return emit();
      }
      if (sec === s.section && !s.series) return;
      s.section = sec;
      s.series = null;
      s.group = 0;
      s.query = "";
      s.sel = 0;
      say(`Showing ${sec}`);
      emit();
    },
    openSeries(item) {
      s.seriesSel = s.sel;
      s.series = item;
      s.group = 0;
      s.query = "";
      s.sel = 0;
      say(`${item.name}: ${plural(item.episodes.length, "episode")}`);
      emit();
    },
    back() {
      if (!s.series) return;
      s.series = null;
      s.group = 0;
      s.query = "";
      s.sel = s.seriesSel;
      clamp();
      emit();
    },
    activate() {
      const item = selected();
      if (!item || busy()) return;
      if (item.kind === "series") act.openSeries(item);
      else act.play(item);
    },
    play(item) {
      if (!item || busy()) return;
      // Switching while something plays reuses the same mpv stream and slot.
      if (s.playing) return act.start(item);
      const a = acct();
      if (a.active >= a.limit) {
        s.dialog = { kind: a.mode === "block" ? "blocked" : "confirm", item };
        return emit();
      }
      act.start(item);
    },
    // `keep` carries the tracks and position over when the stream moves to
    // mpv's own window; anything else starts the stream fresh.
    start(item, keep) {
      s.dialog = null;
      if (item.offline) {
        say(`Could not play ${item.name}: the stream failed to open.`, "err");
        return emit();
      }
      const tr = tracks(item);
      const def = tr.audio.find((a) => a.def) || tr.audio[0];
      s.playing = keep
        ? { ...keep, item, paused: false }
        : { item, paused: false, pos: 0, edge: 0, aid: def ? def.id : 0, sid: 0 };
      say(keep ? `Playing ${item.name} in an mpv window` : `Playing ${item.name}`);
      emit();
    },
    // "Open in mpv window": replays the stream there like a channel switch,
    // so the stream slot is kept and only one stream is ever open.
    detach() {
      const p = s.playing;
      if (!p || s.detached || busy()) return;
      s.detached = true;
      act.start(p.item, p);
    },
    seek(to) {
      const p = s.playing;
      if (!p) return;
      const d = duration(p.item);
      // live streams only seek within what mpv has cached
      p.pos = d ? Math.max(0, Math.min(to, d)) : Math.max(Math.max(0, p.edge - 60), Math.min(to, p.edge));
      emit();
    },
    seekBy(d) {
      if (s.playing) act.seek(s.playing.pos + d);
    },
    setVolume(v) {
      s.volume = Math.max(0, Math.min(130, Math.round(v)));
      s.muted = false;
      emit();
    },
    toggleMute() {
      s.muted = !s.muted;
      emit();
    },
    setTrack(kind, id) {
      if (!s.playing) return;
      s.playing[kind === "audio" ? "aid" : "sid"] = id;
      emit();
    },
    answer(yes) {
      const d = s.dialog;
      s.dialog = null;
      if (d && d.kind === "confirm" && yes) return act.start(d.item);
      say("Cancelled. Nothing was opened.");
      emit();
    },
    pause() {
      if (!s.playing) return;
      s.playing.paused = !s.playing.paused;
      say(s.playing.paused ? "Paused" : `Playing ${s.playing.item.name}`);
      emit();
    },
    stop() {
      if (!s.playing) return;
      s.playing = null;
      s.detached = false;
      say("Stopped. mpv closed and the stream slot is free.");
      emit();
    },
    switchAccount(i) {
      if (s.quit) return;
      if (i === s.account) return emit();
      const target = D.accounts[i];
      s.playing = null;
      s.detached = false;
      s.account = i;
      s.section = "live";
      s.series = null;
      s.group = 0;
      s.query = "";
      s.sel = 0;
      const n = target.m3u ? D.sample.length : D.channels.length;
      say(`Loaded ${plural(n, "channel")} · limit ${plural(target.limit, "stream")}`);
      emit();
    },
    refresh() {
      if (s.quit) return;
      s.playing = null;
      s.detached = false;
      s.section = "live";
      s.series = null;
      s.group = 0;
      s.sel = 0;
      say(`Refreshed ${acct().name}`);
      emit();
    },
    toggleUrl() {
      s.showUrl = !s.showUrl;
      emit();
    },
    toggleDetails() {
      s.showDetails = !s.showDetails;
      emit();
    },
    quit() {
      s.playing = null;
      s.detached = false;
      s.quit = true;
      emit();
    },
    restart() {
      s.quit = false;
      say("Ready. Press Enter on a channel to play.");
      emit();
    },
    setView(v) {
      s.view = v;
      emit();
      Demo.focus();
    },
  };

  // The stream URL embeds the account password, so it is hidden by default.
  const streamUrl = (item) => {
    const host = item.host || acct().server;
    return `${acct().m3u ? "https" : "http"}://${host}/${item.path || ""}`;
  };
  const server = (item) => item.host || acct().server;

  // ------------------------------------------------------------------ video
  // Stands in for the decoded picture: painted on a canvas, never streamed.
  // The playback clock lives in s.playing.pos, so the embedded video and the
  // mpv window show the same moment when the stream moves between them.
  const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const bars = [0, 55, 110, 165, 220, 275, 330];
  const paint = (canvas, p) => {
    const ctx = canvas.getContext("2d");
    const { item } = p;
    const W = canvas.width;
    const H = canvas.height;
    const t = p.pos * 1000;
    const hue = item.hue;
    const g = ctx.createLinearGradient(0, 0, W, H);
    g.addColorStop(0, `hsl(${hue} 55% 24%)`);
    g.addColorStop(1, `hsl(${(hue + 50) % 360} 60% 10%)`);
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, W, H);
    if (isAudio(item)) {
      const n = 28;
      for (let i = 0; i < n; i++) {
        const v = 0.25 + 0.7 * Math.abs(Math.sin(t / 380 + i * 0.7) * Math.cos(t / 900 + i * 0.3));
        ctx.fillStyle = `hsl(${(hue + i * 6) % 360} 70% 62%)`;
        const bh = v * H * 0.55;
        ctx.fillRect(14 + i * ((W - 28) / n), H * 0.72 - bh, (W - 28) / n - 4, bh);
      }
    } else {
      bars.forEach((b, i) => {
        ctx.fillStyle = `hsl(${(hue + b) % 360} 65% 55% / 0.85)`;
        ctx.fillRect((i * W) / 7, 0, W / 7 + 1, H * 0.5);
      });
      for (let i = 0; i < 14; i++) {
        ctx.fillStyle = `hsl(${hue} 20% ${8 + i * 6}%)`;
        ctx.fillRect((i * W) / 14, H * 0.5, W / 14 + 1, H * 0.08);
      }
      // a dot drifting across, so it is visibly "moving"
      const x = ((reduce ? 0 : t / 6) % (W + 40)) - 20;
      ctx.fillStyle = "rgba(255,255,255,.9)";
      ctx.beginPath();
      ctx.arc(x, H * 0.64 + Math.sin(t / 400) * 8, 7, 0, 7);
      ctx.fill();
    }
    // channel card
    ctx.fillStyle = "rgba(8,8,16,.72)";
    ctx.beginPath();
    ctx.roundRect(W * 0.2, H * 0.3, W * 0.6, H * 0.34, 14);
    ctx.fill();
    ctx.fillStyle = "#fff";
    ctx.textAlign = "center";
    ctx.font = "700 40px ui-monospace, monospace";
    ctx.fillText(item.logo || (item.kind === "episode" ? "TV" : item.kind === "movie" ? "FILM" : "♪"), W / 2, H * 0.46);
    ctx.font = "600 15px system-ui, sans-serif";
    ctx.fillText((item.show ? `${item.show} · ` : "") + item.name.replace(/ \(.*\)$/, ""), W / 2, H * 0.57);
    // subtitles, when a track is on
    if (p.sid) {
      const sub = tracks(item).subs.find((x) => x.id === p.sid);
      const line = { eng: "[quiet music]", tur: "[sakin müzik]", spa: "[música suave]" }[sub ? sub.lang : ""] || "…";
      ctx.font = "600 17px system-ui, sans-serif";
      ctx.lineWidth = 4;
      ctx.strokeStyle = "rgba(0,0,0,.85)";
      ctx.strokeText(line, W / 2, H * 0.86);
      ctx.fillText(line, W / 2, H * 0.86);
    }
    // corner tags
    ctx.textAlign = "left";
    ctx.font = "600 13px ui-monospace, monospace";
    ctx.fillStyle = "rgba(255,255,255,.85)";
    ctx.fillText(item.kind === "live" ? "● LIVE" : clock(p.pos), 14, H - 14);
    ctx.textAlign = "right";
    ctx.fillText(item.format, W - 14, H - 14);
    if (p.paused) {
      ctx.fillStyle = "rgba(0,0,0,.45)";
      ctx.fillRect(0, 0, W, H);
      ctx.fillStyle = "#fff";
      ctx.fillRect(W / 2 - 22, H / 2 - 28, 14, 56);
      ctx.fillRect(W / 2 + 8, H / 2 - 28, 14, 56);
    }
  };

  // One clock for whichever player shows the stream. Each player registers
  // { shown(), tick() }; tick paints and updates its time label.
  const Ticker = (() => {
    const players = [];
    let raf = 0;
    let last = 0;
    const frame = (now) => {
      raf = 0;
      const p = s.playing;
      if (!p) return;
      const dt = last ? Math.min(now - last, 250) / 1000 : 0;
      last = now;
      p.edge += dt;
      if (!p.paused) {
        p.pos += dt;
        const d = duration(p.item);
        if (d && p.pos >= d) {
          p.pos = d;
          p.paused = true;
        }
      }
      players.forEach((pl) => pl.shown() && pl.tick());
      if (!p.paused) raf = requestAnimationFrame(frame);
      else last = 0;
    };
    // After every state change: paint once, and keep going while playing.
    subs.push(() => {
      if (!s.playing) {
        cancelAnimationFrame(raf);
        raf = 0;
        last = 0;
      } else if (!raf) raf = requestAnimationFrame(frame);
    });
    return { add: (pl) => players.push(pl) };
  })();

  // -------------------------------------------------------------- mpv window
  // mpv's own window: the terminal UI always plays here, the desktop app
  // after "Open in mpv window".
  const Mpv = (() => {
    const canvas = h("canvas", { width: 480, height: 270, "aria-hidden": "true" });
    const title = h("span", { class: "mpv-title" });
    const pauseBtn = h("button", { class: "mpv-btn", "aria-label": "Pause or resume", onclick: () => act.pause() });
    const time = h("span", { class: "mpv-time" });
    const root = h(
      "div",
      { class: "mpv", hidden: true, role: "group", "aria-label": "mpv window (mock)" },
      h(
        "div",
        { class: "mpv-bar" },
        title,
        h("button", { class: "mpv-btn mpv-x", "aria-label": "Close mpv", onclick: () => act.stop() }, icon("close")),
      ),
      canvas,
      h("div", { class: "mpv-ctl" }, pauseBtn, time),
    );
    const shown = () => !!s.playing && (s.view === "tui" || s.detached);

    const render = () => {
      root.hidden = !shown();
      if (root.hidden) return;
      const p = s.playing;
      title.textContent = `mpv · ${p.item.name}`;
      pauseBtn.replaceChildren(icon(p.paused ? "play" : "pause"));
    };
    const tick = () => {
      paint(canvas, s.playing);
      time.textContent = s.playing.item.kind === "live" ? "LIVE" : clock(s.playing.pos);
    };

    // Drag by the title bar, like moving a separate window.
    const bar = root.querySelector(".mpv-bar");
    bar.addEventListener("pointerdown", (e) => {
      if (e.target.closest("button")) return;
      const stage = root.parentElement.getBoundingClientRect();
      const r = root.getBoundingClientRect();
      const dx = e.clientX - r.left;
      const dy = e.clientY - r.top;
      bar.setPointerCapture(e.pointerId);
      const move = (ev) => {
        const x = Math.max(0, Math.min(stage.width - r.width, ev.clientX - stage.left - dx));
        const y = Math.max(0, Math.min(stage.height - r.height, ev.clientY - stage.top - dy));
        root.style.left = `${x}px`;
        root.style.top = `${y}px`;
        root.style.right = "auto";
        root.style.bottom = "auto";
      };
      const up = () => {
        bar.removeEventListener("pointermove", move);
        bar.removeEventListener("pointerup", up);
      };
      bar.addEventListener("pointermove", move);
      bar.addEventListener("pointerup", up);
    });

    subs.push(render);
    Ticker.add({ shown, tick });
    return { root };
  })();

  // ---------------------------------------------------------- embedded video
  // tuner-gtk's video pane (docs/gui.md#embedded-video): libmpv draws into the
  // window and tuner draws its own control bar over it, themed like the rest.
  const Video = (() => {
    const v = { cinema: false, full: false, info: false, stats: false, menu: false };
    const canvas = h("canvas", { width: 640, height: 360, class: "v-canvas", "aria-hidden": "true" });
    const badge = h("span", { class: "v-info", hidden: true });
    const statsBox = h("pre", { class: "v-stats", hidden: true, "aria-label": "mpv statistics" });

    const btn = (name, label, onclick, cls = "") =>
      h("button", { class: `v-btn ${cls}`.trim(), type: "button", title: label, "aria-label": label, onclick }, icon(name));
    const label = (b, text) => (b.title = text, b.setAttribute("aria-label", text));
    const range = (cls, name, max, oninput) =>
      h("input", { type: "range", class: `v-range ${cls}`, min: 0, max, step: 1, value: 0, "aria-label": name, oninput });
    const fill = (r) => r.style.setProperty("--p", `${(100 * r.value) / (r.max || 1)}%`);

    const seek = range("v-seek", "Seek", 1, (e) => act.seek(+e.target.value));
    const vol = range("v-vol", "Volume", 130, (e) => act.setVolume(+e.target.value));
    const time = h("span", { class: "v-time" });
    const pauseBtn = btn("pause", "Pause", () => act.pause(), "v-play");
    const muteBtn = btn("volume", "Mute", () => act.toggleMute());
    const tracksBtn = btn("subtitles", "Audio & subtitles", () => setMenu(!v.menu), "v-tracks");
    tracksBtn.setAttribute("aria-haspopup", "true");
    const infoBtn = btn("eye", "Stream info: resolution, frame rate, bitrate", () => toggle("info"));
    const statsBtn = btn("stats", "Playback statistics", () => toggle("stats"));
    const detachBtn = btn("detach", "Open in mpv window", () => act.detach());
    const cinemaBtn = btn("cinema", "Cinema mode", () => setCinema(!v.cinema));
    const fullBtn = btn("full", "Fullscreen (F11)", () => setFull(!v.full));
    const menu = h("div", { class: "v-menu", hidden: true, role: "menu", "aria-label": "Audio & subtitles" });
    // Seek bar across the top; below it playback, volume and time on the
    // left, everything about the view on the right.
    const ctl = h(
      "div",
      { class: "v-ctl" },
      seek,
      h(
        "div",
        { class: "v-row" },
        pauseBtn,
        btn("rewind", "Back 10 s", () => act.seekBy(-10)),
        btn("forward", "Forward 10 s", () => act.seekBy(10)),
        muteBtn,
        vol,
        time,
        h("span", { class: "grow" }),
        tracksBtn,
        infoBtn,
        statsBtn,
        detachBtn,
        cinemaBtn,
        fullBtn,
      ),
    );
    const root = h("section", { class: "g-video", hidden: true, "aria-label": "Video" }, canvas, badge, statsBox, ctl, menu);

    const shown = () => !!s.playing && !s.detached && s.view === "gui";

    // The bar fades in on pointer movement and out 2.5 s later, unless the
    // track menu is open (or, here, the keyboard is in it).
    let hideTimer = 0;
    const reveal = () => {
      ctl.classList.add("show");
      clearTimeout(hideTimer);
      hideTimer = setTimeout(() => {
        if (!v.menu && !ctl.querySelector(":focus-visible")) ctl.classList.remove("show");
      }, 2500);
    };
    root.addEventListener("pointermove", reveal);
    root.addEventListener("pointerdown", reveal);
    ctl.addEventListener("focusin", reveal);
    // mpv's default mouse bindings: double-click fullscreen, wheel seeks.
    canvas.addEventListener("dblclick", () => setFull(!v.full));
    canvas.addEventListener(
      "wheel",
      (e) => {
        if (!Demo.active) return;
        e.preventDefault();
        act.seekBy(e.deltaY < 0 ? 10 : -10);
      },
      { passive: false },
    );
    document.addEventListener("pointerdown", (e) => {
      if (v.menu && !e.target.closest(".v-menu, .v-tracks")) setMenu(false);
    });

    const toggle = (k) => {
      v[k] = !v[k];
      render();
    };
    const setMenu = (open) => {
      v.menu = open;
      render();
      reveal();
    };
    const setCinema = (on) => {
      v.cinema = on;
      emit();
    };
    // Fullscreen shows only the video and its bar: the demo's stage goes
    // fullscreen when the browser allows it, and leaving it there (Esc)
    // leaves fullscreen here too.
    const setFull = (on, quiet) => {
      v.full = on;
      try {
        const stage = root.closest(".stage");
        if (on && stage && stage.requestFullscreen && !document.fullscreenElement) stage.requestFullscreen().catch(() => {});
        else if (!on && document.fullscreenElement) document.exitFullscreen().catch(() => {});
      } catch {
        // no Fullscreen API: the window still shows only the video
      }
      if (!quiet) emit();
    };
    document.addEventListener("fullscreenchange", () => {
      if (!document.fullscreenElement && v.full) {
        v.full = false;
        emit();
      }
    });

    const trackRow = (kind, t, cur) =>
      h(
        "button",
        {
          class: `v-track ${cur ? "on" : ""}`,
          type: "button",
          role: "menuitemradio",
          "aria-checked": String(cur),
          onclick: () => {
            v.menu = false;
            act.setTrack(kind, t ? t.id : 0);
          },
        },
        h(
          "span",
          { class: "v-track-main" },
          h("b", { text: t ? LANGS[t.lang] || t.lang : "Off" }),
          t
            ? h("small", {
                text: [t.title, t.codec, kind === "audio" ? t.ch : null, t.def ? "Default" : null, t.forced ? "Forced" : null].filter(Boolean).join(" · "),
              })
            : null,
        ),
        cur ? icon("check") : null,
      );
    const renderMenu = () => {
      const p = s.playing;
      const tr = tracks(p.item);
      menu.replaceChildren(
        h("div", { class: "v-col" }, h("h4", { text: "Audio" }), ...tr.audio.map((t) => trackRow("audio", t, t.id === p.aid))),
        h("div", { class: "v-col" }, h("h4", { text: "Subtitles" }), trackRow("sub", null, !p.sid), ...tr.subs.map((t) => trackRow("sub", t, t.id === p.sid))),
      );
    };

    // What mpv's stats overlay shows on its first page.
    const statsText = (p) => {
      const it = p.item;
      const pr = params(it);
      const a = tracks(it).audio.find((x) => x.id === p.aid);
      return [
        `File: ${(it.path || "").split("/").pop()}`,
        pr ? `Video: h264  ${pr.w}x${pr.h}  ${pr.fps.toFixed(3)} fps  ${pr.mbps.toFixed(1)} Mbps` : null,
        a ? `Audio: ${a.codec.toLowerCase()}  ${a.ch}  48000 Hz` : null,
        `Cache: ${(4 + (it.hue % 5) + (p.paused ? 6 : 0)).toFixed(1)}s`,
      ]
        .filter(Boolean)
        .join("\n");
    };

    const tick = () => {
      const p = s.playing;
      paint(canvas, p);
      const d = duration(p.item);
      time.textContent = d ? `${clock(p.pos)} / ${clock(d)}` : clock(p.pos);
      if (d && !seek.matches(":active")) {
        seek.value = p.pos;
        fill(seek);
      }
    };

    const render = () => {
      root.hidden = !shown();
      if (root.hidden) {
        // Nothing left to watch (stopped, or moved to mpv's window): leave
        // cinema mode and fullscreen, which would otherwise show an empty window.
        v.cinema = false;
        v.menu = false;
        if (v.full) setFull(false, true);
        return;
      }
      const p = s.playing;
      const d = duration(p.item);
      pauseBtn.replaceChildren(icon(p.paused ? "play" : "pause"));
      label(pauseBtn, p.paused ? "Play" : "Pause");
      muteBtn.replaceChildren(icon(s.muted ? "muted" : "volume"));
      label(muteBtn, s.muted ? "Unmute" : "Mute");
      vol.value = s.volume;
      fill(vol);
      // live streams have no length, so no seek bar
      seek.hidden = !d;
      if (d) seek.max = d;
      for (const [b, on] of [[infoBtn, v.info], [statsBtn, v.stats], [cinemaBtn, v.cinema], [tracksBtn, v.menu]]) {
        b.classList.toggle("on", on);
        b.setAttribute(b === tracksBtn ? "aria-expanded" : "aria-pressed", String(on));
      }
      fullBtn.replaceChildren(icon(v.full ? "unfull" : "full"));
      label(fullBtn, v.full ? "Leave fullscreen (F11)" : "Fullscreen (F11)");
      const pr = params(p.item);
      badge.hidden = !v.info;
      badge.textContent = pr ? `${pr.w}×${pr.h} · ${+pr.fps.toFixed(2)} fps · ${pr.mbps.toFixed(1)} Mb/s` : "Audio only";
      statsBox.hidden = !v.stats;
      statsBox.classList.toggle("below", v.info);
      statsBox.textContent = statsText(p);
      menu.hidden = !v.menu;
      if (v.menu) renderMenu();
      tick();
    };

    subs.push(render);
    Ticker.add({ shown, tick });
    return { root, shown, state: v, setFull, setCinema };
  })();

  // --------------------------------------------------------------- GTK view
  const Gtk = (() => {
    let mode = "list"; // list | cards
    let pop = null; // acct | theme | null
    let lastSel = null;
    // The details panel closes while the video is open, and comes back as
    // it was when the video closes.
    let videoWas = false;
    let detailsBefore = null;

    const search = h("input", {
      type: "search",
      class: "g-search-in",
      placeholder: "Search channels",
      "aria-label": "Search channels",
      autocomplete: "off",
      spellcheck: "false",
      oninput: (e) => act.setQuery(e.target.value),
    });
    const acctBtn = h("button", { class: "g-btn g-acct", "aria-haspopup": "true", onclick: () => togglePop("acct") });
    const themeBtn = h(
      "button",
      { class: "g-btn g-icon", title: "Theme", "aria-label": "Theme picker", onclick: () => togglePop("theme") },
      icon("palette"),
    );
    const refreshBtn = h("button", { class: "g-btn g-icon", title: "Refresh (F5)", "aria-label": "Refresh", onclick: () => act.refresh() }, icon("refresh"));
    const infoBtn = h("button", { class: "g-btn g-icon g-info", title: "Details (Ctrl+I)", "aria-label": "Toggle details", onclick: () => act.toggleDetails() }, icon("info"));
    const popover = h("div", { class: "g-pop", hidden: true });

    const head = h(
      "header",
      { class: "g-head" },
      acctBtn,
      refreshBtn,
      h("label", { class: "g-search" }, icon("search"), search),
      infoBtn,
      themeBtn,
      popover,
    );

    const seg = h("div", { class: "g-seg", role: "tablist" });
    const sideHead = h("h3", { class: "g-sidehead" });
    const groupList = h("ul", { class: "g-groups" });
    const side = h("aside", { class: "g-side" }, seg, sideHead, groupList);

    const backBtn = h("button", { class: "g-btn g-back", onclick: () => act.back() }, icon("back"), "Series");
    const mainTitle = h("h2", { class: "g-title" });
    const count = h("span", { class: "g-count" });
    const listBtn = h("button", { class: "g-btn g-icon g-view", title: "List view", "aria-label": "List view", onclick: () => setMode("list") }, icon("list"));
    const cardsBtn = h("button", { class: "g-btn g-icon g-view", title: "Cards view", "aria-label": "Cards view", onclick: () => setMode("cards") }, icon("grid"));
    const mainHead = h("div", { class: "g-mainhead" }, backBtn, mainTitle, count, h("span", { class: "grow" }), listBtn, cardsBtn);
    const list = h("div", { class: "g-list", role: "listbox", "aria-label": "Channels" });
    const main = h("main", { class: "g-main" }, mainHead, list);

    const details = h("aside", { class: "g-details" });

    const playerBar = h("div", { class: "g-player", hidden: true });
    const status = h("footer", { class: "g-status" });
    const overlay = h("div", { class: "g-overlay", hidden: true });

    // Divider between the channel list and the video; drag (or focus and
    // use Left/Right) to resize. The video takes the rest of the width.
    const handle = h("div", { class: "g-handle", role: "separator", "aria-orientation": "vertical", "aria-label": "Resize channel list", tabindex: "0", hidden: true });
    const body = h("div", { class: "g-body" }, side, main, handle, Video.root, details);
    const root = h("div", { class: "gtk", id: "gtk", tabindex: "-1" }, head, body, playerBar, status, overlay);

    let listW = 300;
    const setListW = (w) => {
      const room = body.clientWidth - side.offsetWidth - (details.hidden ? 0 : details.offsetWidth) - handle.offsetWidth;
      listW = Math.round(Math.max(220, Math.min(w, room - 240)));
      root.style.setProperty("--list-w", `${listW}px`);
    };
    handle.addEventListener("pointerdown", (e) => {
      const x0 = e.clientX;
      const w0 = main.offsetWidth;
      handle.setPointerCapture(e.pointerId);
      const move = (ev) => setListW(w0 + ev.clientX - x0);
      const up = () => {
        handle.removeEventListener("pointermove", move);
        handle.removeEventListener("pointerup", up);
      };
      handle.addEventListener("pointermove", move);
      handle.addEventListener("pointerup", up);
    });
    handle.addEventListener("keydown", (e) => {
      if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
      setListW(listW + (e.key === "ArrowRight" ? 10 : -10));
      e.preventDefault();
      e.stopPropagation();
    });

    const setMode = (m) => {
      mode = m;
      emit();
    };
    const togglePop = (p) => {
      pop = pop === p ? null : p;
      renderPop();
    };
    document.addEventListener("pointerdown", (e) => {
      if (pop && !e.target.closest(".g-pop, .g-acct, .g-icon")) {
        pop = null;
        renderPop();
      }
    });

    const renderPop = () => {
      popover.hidden = !pop;
      popover.className = `g-pop g-pop-${pop || ""}`;
      acctBtn.setAttribute("aria-expanded", String(pop === "acct"));
      if (pop === "acct") {
        popover.replaceChildren(
          h("h4", { text: "My playlists" }),
          ...D.accounts.map((a, i) =>
            h(
              "button",
              { class: `g-pop-row ${i === s.account ? "on" : ""}`, onclick: () => ((pop = null), act.switchAccount(i)) },
              h("span", { class: "g-pop-main" }, h("b", { text: a.name }), h("small", { text: a.type })),
              i === s.account ? icon("check") : null,
            ),
          ),
          h("p", { class: "g-pop-note", text: "Add or manage accounts is in the real app (Ctrl+,)." }),
        );
      } else if (pop === "theme") {
        popover.replaceChildren(
          h("h4", { text: "Theme" }),
          h(
            "div",
            { class: "g-pop-scroll" },
            ...Theme.all.map((t) =>
              h(
                "button",
                {
                  class: `g-pop-row ${t.name === Theme.demo.name ? "on" : ""}`,
                  onclick: () => {
                    Theme.setDemo(t.name);
                    pop = null;
                    emit();
                  },
                },
                h("span", { class: "swatches" }, ...[t.bg, t.accent, t.fg, t.err].map((c) => h("i", { style: `background:${c}` }))),
                h("span", { class: "g-pop-main" }, h("b", { text: t.label })),
                t.name === Theme.demo.name ? icon("check") : null,
              ),
            ),
          ),
        );
      }
    };

    const row = (item, i) => {
      const playing = s.playing && s.playing.item.id === item.id;
      let sub = item.group;
      if (item.kind === "live" && item.catchup) sub += ` · ${item.catchup}d replay`;
      if (item.kind === "series") sub = `${item.year} · ★ ${item.rating}`;
      if (item.kind === "movie") sub = `${item.group} · ${item.year} · ★ ${item.rating}`;
      if (item.kind === "episode") sub = `${item.show}`;
      const badge = playing ? (s.playing.paused ? "Ⅱ Paused" : "▶ Playing") : "";
      const common = {
        role: "option",
        "aria-selected": String(i === s.sel),
        class: `${mode === "cards" ? "g-card" : "g-row"} ${i === s.sel ? "sel" : ""} ${playing ? "playing" : ""}`,
        onclick: () => {
          act.setSel(i);
          if (touch.matches) act.activate();
        },
        ondblclick: () => act.activate(),
      };
      if (mode === "cards") {
        return h(
          "div",
          common,
          logo(item, "logo-lg"),
          h("b", { class: "g-name", text: item.name }),
          h("small", { text: sub }),
          badge ? h("span", { class: "g-badge", text: badge }) : null,
        );
      }
      return h(
        "div",
        common,
        h("span", { class: "g-num", text: item.kind === "live" ? item.num : "" }),
        logo(item),
        h("span", { class: "g-rowtxt" }, h("b", { class: "g-name", text: item.name }), h("small", { text: sub })),
        badge ? h("span", { class: "g-badge", text: badge }) : null,
      );
    };

    const renderSide = () => {
      const a = acct();
      seg.replaceChildren(
        ...[
          ["live", "Live"],
          ["movies", "Movies"],
          ["series", "Series"],
        ].map(([k, label]) =>
          h(
            "button",
            {
              role: "tab",
              "aria-selected": String(s.section === k),
              class: `${s.section === k ? "on" : ""} ${k !== "live" && !a.vod ? "off" : ""}`,
              title: k !== "live" && !a.vod ? "Needs an Xtream login" : `${label} (Ctrl+${["live", "movies", "series"].indexOf(k) + 1})`,
              onclick: () => act.setSection(k),
              text: label,
            },
          ),
        ),
      );
      sideHead.textContent = s.series ? "Seasons" : { live: "Your channels", movies: "Categories", series: "Categories" }[s.section];
      groupList.replaceChildren(
        ...groups().map((g, i) =>
          h(
            "li",
            { class: i === s.group ? "on" : "", onclick: () => act.setGroup(i) },
            h("span", { text: g.label }),
            h("small", { text: g.count }),
          ),
        ),
      );
    };

    const renderMain = () => {
      const g = groups()[s.group];
      const its = items();
      backBtn.hidden = !s.series;
      mainTitle.textContent = s.series ? s.series.name : g.label;
      const noun = s.series ? "episode" : { live: "channel", movies: "movie", series: "series" }[s.section];
      count.textContent = s.section === "series" && !s.series ? `${its.length} series` : plural(its.length, noun);
      search.placeholder = `Search ${{ live: "channels", movies: "movies", series: "series" }[s.section]}`;
      if (search.value !== s.query) search.value = s.query;
      listBtn.classList.toggle("on", mode === "list");
      cardsBtn.classList.toggle("on", mode === "cards");
      list.className = `g-list ${mode === "cards" ? "cards" : ""}`;
      const top = list.scrollTop;
      if (!its.length) {
        list.replaceChildren(h("p", { class: "g-empty", text: s.query ? `No channels match “${s.query}”.` : "Nothing here." }));
      } else {
        list.replaceChildren(...its.map(row));
      }
      list.scrollTop = top;
      const cur = selected();
      const key = cur ? cur.id : null;
      if (key !== lastSel) {
        keepInView(list, list.querySelector(".sel"));
        lastSel = key;
      }
    };

    const field = (k, v) => [h("dt", { text: k }), h("dd", { text: v })];
    const renderDetails = () => {
      const it = selected();
      details.hidden = !s.showDetails;
      infoBtn.classList.toggle("on", s.showDetails);
      if (!it) {
        details.replaceChildren(h("p", { class: "g-empty", text: "Select something to see its details." }));
        return;
      }
      const rows = [];
      if (it.kind === "live") {
        rows.push(
          ...field("Group", it.group),
          ...field("Number", String(it.num)),
          ...field("Catch-up", it.catchup ? plural(it.catchup, "day") : "none"),
          ...field("EPG id", it.epg || "none"),
          ...field("Format", it.format),
          ...field("Server", server(it)),
        );
      } else if (it.kind === "series") {
        rows.push(...field("Category", it.group), ...field("Year", String(it.year)), ...field("Rating", `★ ${it.rating}`), ...field("Seasons", String(it.seasons)));
      } else {
        rows.push(
          ...field(it.kind === "episode" ? "Season" : "Category", it.group),
          ...field("Year", String(it.year)),
          ...field("Rating", `★ ${it.rating}`),
          ...field("Format", it.format),
          ...field("Server", server(it)),
        );
      }
      const label = { live: "Play channel", movie: "Play movie", episode: "Play episode", series: "Show episodes" }[it.kind];
      details.replaceChildren(
        ...[
          h("div", { class: "g-hero" }, logo(it, "logo-xl")),
          h("h3", { class: "g-dtitle", text: it.name }),
          h("dl", { class: "g-dl" }, ...rows),
          it.plot ? h("p", { class: "g-plot", text: it.plot }) : null,
          h("button", { class: "g-btn g-primary", onclick: () => act.activate() }, label),
          it.kind !== "series"
            ? h(
                "button",
                { class: "g-btn g-ghost", "aria-pressed": String(s.showUrl), onclick: () => act.toggleUrl() },
                s.showUrl ? "Hide stream URL" : "Show stream URL",
              )
            : null,
          it.kind !== "series" && s.showUrl ? h("code", { class: "g-url", text: streamUrl(it) }) : null,
        ].filter(Boolean),
      );
    };

    const renderPlayer = () => {
      const p = s.playing;
      playerBar.hidden = !p;
      if (!p) return;
      const it = p.item;
      const meta = it.kind === "live" ? `#${it.num} · ${it.group} · ${it.format}` : `${it.show ? it.show + " · " : ""}${it.group} · ${it.format}`;
      playerBar.replaceChildren(
        logo(it, "logo-md"),
        h("div", { class: "g-ptxt" }, h("b", { text: it.name }), h("small", { text: meta })),
        h("span", { class: "grow" }),
        h("span", { class: "g-pstate", text: p.paused ? "Paused" : "Playing in mpv" }),
        h("button", { class: "g-round", title: "Pause / resume (Ctrl+P)", "aria-label": p.paused ? "Resume" : "Pause", onclick: () => act.pause() }, icon(p.paused ? "play" : "pause")),
        h("button", { class: "g-round", title: "Stop (Ctrl+S)", "aria-label": "Stop", onclick: () => act.stop() }, icon("stop")),
      );
    };

    const renderStatus = () => {
      status.replaceChildren(h("span", { class: `g-st ${s.status.kind}`, text: s.status.text }), h("span", { class: "grow" }), h("span", { class: "g-sum", text: summary() }));
    };

    const renderOverlay = () => {
      const d = s.dialog;
      overlay.hidden = !(d || s.quit);
      if (overlay.hidden) return;
      if (d) {
        const a = acct();
        const blocked = d.kind === "blocked";
        overlay.replaceChildren(
          h(
            "div",
            { class: "g-dialog", role: "alertdialog", "aria-modal": "true", "aria-label": blocked ? "Stream limit reached" : "Stream limit" },
            h("h3", { text: blocked ? "Stream limit reached" : "Stream limit" }),
            h(
              "p",
              {
                text: blocked
                  ? `“${a.name}” allows ${plural(a.limit, "stream")} and the provider reports ${a.active} in use. Nothing was opened. This account is set to never play when full.`
                  : `The provider reports ${a.active} of ${a.limit} connections in use on “${a.name}”, possibly on another device. Playing anyway may get the account suspended.`,
              },
            ),
            h(
              "div",
              { class: "g-dlg-btns" },
              blocked
                ? h("button", { class: "g-btn g-primary", autofocus: true, onclick: () => act.answer(false) }, "Close")
                : [
                    h("button", { class: "g-btn g-primary", onclick: () => act.answer(false) }, "Cancel"),
                    h("button", { class: "g-btn g-danger", onclick: () => act.answer(true) }, "Play anyway"),
                  ],
            ),
          ),
        );
        const first = overlay.querySelector("button");
        if (first && Demo.active) first.focus();
      } else {
        overlay.replaceChildren(
          h("div", { class: "g-dialog" }, h("h3", { text: "Tuner closed" }), h("p", { text: "Closing the window quits mpv and frees the stream slot." }), h("button", { class: "g-btn g-primary", onclick: () => act.restart() }, "Open Tuner again")),
        );
      }
    };

    const render = () => {
      clamp();
      const video = Video.shown();
      if (s.view === "gui" && video !== videoWas) {
        if (video) {
          detailsBefore = s.showDetails;
          s.showDetails = false;
        } else if (detailsBefore !== null) {
          s.showDetails = detailsBefore;
          detailsBefore = null;
        }
        videoWas = video;
      }
      handle.hidden = !video;
      root.classList.toggle("has-video", video);
      root.classList.toggle("no-details", !s.showDetails);
      root.classList.toggle("cinema", video && Video.state.cinema);
      root.classList.toggle("full", video && Video.state.full);
      acctBtn.replaceChildren(h("span", { text: acct().name }), icon("chevron"));
      renderSide();
      renderMain();
      renderDetails();
      renderPlayer();
      renderStatus();
      renderOverlay();
      renderPop();
    };

    const key = (e) => {
      const ctrl = e.ctrlKey || e.metaKey;
      const k = e.key;
      if (s.dialog) {
        if (k === "Escape" || k === "Enter") {
          act.answer(false);
          return true;
        }
        return k !== "Tab";
      }
      if (s.quit) return false;
      // The video's own keys: F11 fullscreen; in fullscreen Esc leaves and
      // Space pauses; in cinema mode Esc leaves it.
      const v = Video.state;
      if (Video.shown()) {
        if (k === "F11") return Video.setFull(!v.full), true;
        if (v.full && k === "Escape") return Video.setFull(false), true;
        if (v.full && k === " ") return act.pause(), true;
        if (v.cinema && k === "Escape") return Video.setCinema(false), true;
      }
      if (document.activeElement && document.activeElement.matches("input[type=range]") && k.startsWith("Arrow")) return false;
      if (k === "Escape") {
        if (pop) {
          pop = null;
          renderPop();
        } else if (s.query) act.setQuery("");
        else if (s.series) act.back();
        return true;
      }
      if (ctrl && !e.altKey) {
        const map = { f: () => search.focus(), p: () => act.pause(), s: () => act.stop(), i: () => act.toggleDetails(), t: () => togglePop("theme"), r: () => act.refresh(), o: () => togglePop("acct"), 1: () => act.setSection("live"), 2: () => act.setSection("movies"), 3: () => act.setSection("series") };
        if (map[k.toLowerCase()]) {
          map[k.toLowerCase()]();
          return true;
        }
        return false;
      }
      if (e.altKey) {
        if (k === "ArrowLeft") return act.back(), true;
        const n = { 1: "live", 2: "movies", 3: "series" }[k];
        if (n) return act.setSection(n), true;
        return false;
      }
      if (k === "F5") return act.refresh(), true;
      if (k === "ArrowDown" || k === "ArrowUp") {
        act.move(k === "ArrowDown" ? 1 : -1);
        return true;
      }
      if (mode === "cards" && (k === "ArrowRight" || k === "ArrowLeft") && document.activeElement !== search) {
        act.move(k === "ArrowRight" ? 1 : -1);
        return true;
      }
      if (k === "PageDown" || k === "PageUp") return act.move(k === "PageDown" ? 8 : -8), true;
      if (k === "Enter") {
        if (document.activeElement && document.activeElement.closest(".g-pop, .g-btn, .g-seg, .g-dialog, .v-ctl, .g-handle")) return false;
        act.activate();
        return true;
      }
      // type anywhere: focus jumps to the search box
      if (k.length === 1 && document.activeElement !== search && !document.activeElement.closest("button, input")) {
        search.focus();
      }
      return false;
    };

    subs.push(render);
    return { root, key, focusSearch: () => search.focus() };
  })();

  // --------------------------------------------------------------- TUI view
  const Tui = (() => {
    let focus = "list"; // groups | list
    let mode = "normal"; // normal | search | accounts
    let acctSel = 0;
    let lastSel = null;

    const groupsBox = h("div", { class: "t-lines" });
    const listBox = h("div", { class: "t-lines" });
    const detBox = h("div", { class: "t-lines t-det" });
    const gTitle = h("span", { class: "t-title" });
    const lTitle = h("span", { class: "t-title" });
    const dTitle = h("span", { class: "t-title", text: "Details" });
    const gPane = h("section", { class: "t-box t-groups" }, gTitle, groupsBox);
    const lPane = h("section", { class: "t-box t-list" }, lTitle, listBox);
    const dPane = h("section", { class: "t-box t-detbox" }, dTitle, detBox);
    const stat = h("div", { class: "t-status" });
    const hints = h("div", { class: "t-hints", text: "1-3 section · j/k move · h/l pane · / search · a accounts · i info · u url · enter play · space pause · s stop · q quit" });
    const popup = h("div", { class: "t-pop-wrap", hidden: true });
    const root = h("div", { class: "tui", id: "tui", tabindex: "0", "aria-label": "Tuner terminal UI (mock). Click, then use the keyboard.", hidden: true }, h("div", { class: "t-main" }, gPane, h("div", { class: "t-right" }, lPane, dPane)), stat, hints, popup);

    const line = (cls, ...kids) => h("div", { class: `t-line ${cls || ""}` }, ...kids);

    const renderGroups = () => {
      gTitle.textContent = s.series ? "Seasons" : s.section === "live" ? "Groups" : "Categories";
      groupsBox.replaceChildren(
        ...groups().map((g, i) =>
          h("div", { class: `t-line ${i === s.group ? (focus === "groups" ? "rev" : "dim-rev") : ""}`, onclick: () => (focus = "groups", act.setGroup(i)) }, g.name === null ? "All" : g.label),
        ),
      );
    };

    const renderList = () => {
      const its = items();
      lTitle.textContent = `${s.series ? s.series.name : sectionName()} (${its.length})`;
      const top = listBox.scrollTop;
      listBox.replaceChildren(
        ...(its.length
          ? its.map((it, i) => {
              const playing = s.playing && s.playing.item.id === it.id;
              const mark = playing ? (s.playing.paused ? "Ⅱ " : "▶ ") : "  ";
              const extra = it.kind === "series" ? `  ${it.year}  ★ ${it.rating}` : it.kind === "movie" ? `  ${it.year}  ★ ${it.rating}` : "";
              return h("div", { class: `t-line ${i === s.sel ? (focus === "list" ? "rev" : "dim-rev") : ""} ${playing ? "playing" : ""}`, onclick: () => (focus = "list", act.setSel(i)), ondblclick: () => act.activate() }, mark + it.name + extra);
            })
          : [line("dim", s.query ? `  no match for “${s.query}”` : "  nothing here")]),
      );
      listBox.scrollTop = top;
      const cur = selected();
      const key = cur ? cur.id : null;
      if (key !== lastSel) {
        keepInView(listBox, listBox.querySelector(".rev, .dim-rev"));
        lastSel = key;
      }
      keepInView(groupsBox, groupsBox.querySelector(".rev, .dim-rev"));
    };

    const kv = (k, v, cls) => h("div", { class: `t-line t-kv ${cls || ""}` }, h("span", { class: "t-k", text: k }), h("span", { text: v }));
    const renderDetails = () => {
      dPane.hidden = !s.showDetails;
      const it = selected();
      if (!it) return detBox.replaceChildren(line("dim", "nothing selected"));
      const L = [line("bold", it.name)];
      if (it.kind === "live") {
        L.push(kv("Group", it.group), kv("Number", String(it.num)), kv("Catch-up", it.catchup ? plural(it.catchup, "day") : "none"), kv("EPG id", it.epg || "none"), kv("Format", it.format), kv("Server", server(it)), kv("Logo", "no"));
      } else if (it.kind === "series") {
        L.push(kv("Category", it.group), kv("Year", String(it.year)), kv("Rating", `★ ${it.rating}`), kv("Plot", it.plot));
      } else {
        L.push(kv(it.kind === "episode" ? "Season" : "Category", it.group), kv("Year", String(it.year)), kv("Rating", `★ ${it.rating}`), kv("Format", it.format), kv("Server", server(it)));
      }
      if (it.kind !== "series") L.push(kv("URL", s.showUrl ? streamUrl(it) : "hidden (press u)", s.showUrl ? "warn" : "dim"));
      detBox.replaceChildren(...L);
    };

    const renderStatus = () => {
      if (mode === "search") {
        stat.replaceChildren(h("span", { class: "bold", text: `/${s.query}` }), h("span", { class: "t-cursor", text: "█" }), h("span", { class: "dim", text: "   enter play · esc clear" }));
        return;
      }
      const txt = s.status.text;
      stat.replaceChildren(h("span", { class: `bold ${s.status.kind}`, text: txt }), h("span", { class: "grow" }), h("span", { class: "dim t-sum", text: summary() }));
    };

    const renderPopup = () => {
      popup.hidden = !(mode === "accounts" || s.dialog || s.quit);
      popup.classList.toggle("full", !!s.quit);
      if (popup.hidden) return;
      if (s.quit) {
        popup.replaceChildren(h("div", { class: "t-quit" }, h("div", { class: "dim", text: "$ tuner" }), h("div", { text: "tuner exited. mpv was closed and the stream slot is free." }), h("button", { class: "t-btn", onclick: () => act.restart() }, "$ tuner  ↵")));
        return;
      }
      if (s.dialog) {
        const a = acct();
        const blocked = s.dialog.kind === "blocked";
        popup.replaceChildren(
          h(
            "div",
            { class: "t-pop" },
            h("span", { class: "t-title", text: blocked ? "Stream limit reached" : "Stream limit" }),
            h("p", { text: blocked ? `“${a.name}” allows ${plural(a.limit, "stream")}; the provider reports ${a.active} in use. Nothing was opened.` : `The provider reports ${a.active} of ${a.limit} connections in use on “${a.name}”. Playing anyway may get the account suspended.` }),
            h("p", { class: "dim", text: blocked ? "any key closes" : "y play anyway · any other key cancels" }),
          ),
        );
        return;
      }
      popup.replaceChildren(
        h(
          "div",
          { class: "t-pop" },
          h("span", { class: "t-title", text: "Accounts" }),
          ...D.accounts.map((a, i) => h("div", { class: `t-line ${i === acctSel ? "rev" : ""}`, onclick: () => ((acctSel = i), pickAccount()) }, `${i === s.account ? "● " : "  "}${a.name}`.padEnd(14) + `  ${a.type}`)),
          h("p", { class: "dim", text: "j/k move · enter load · esc close" }),
        ),
      );
    };

    const pickAccount = () => {
      mode = "normal";
      act.switchAccount(acctSel);
    };

    const render = () => {
      clamp();
      renderGroups();
      renderList();
      renderDetails();
      renderStatus();
      renderPopup();
      root.classList.toggle("no-details", !s.showDetails);
      gPane.classList.toggle("focus", focus === "groups");
      lPane.classList.toggle("focus", focus === "list");
    };


    const last = (n) => n - 1;
    const key = (k, ctrl = false) => {
      if (ctrl && k.toLowerCase() === "c") return act.quit(), true;
      if (s.quit) return k === "Enter" ? (act.restart(), true) : false;
            if (s.dialog) {
        if (s.dialog.kind === "confirm" && k === "y") act.answer(true);
        else act.answer(false);
        return true;
      }
      if (mode === "accounts") {
        if (k === "j" || k === "ArrowDown") acctSel = Math.min(acctSel + 1, last(D.accounts.length));
        else if (k === "k" || k === "ArrowUp") acctSel = Math.max(acctSel - 1, 0);
        else if (k === "g" || k === "Home") acctSel = 0;
        else if (k === "G" || k === "End") acctSel = last(D.accounts.length);
        else if (k === "Enter") return pickAccount(), true;
        else if (k === "Escape" || k === "q" || k === "a") mode = "normal";
        else return false;
        render();
        return true;
      }
      if (mode === "search") {
        if (k === "Escape") {
          mode = "normal";
          act.setQuery("");
        } else if (k === "Enter") {
          mode = "normal";
          act.activate();
        } else if (k === "Backspace") act.setQuery(s.query.slice(0, -1));
        else if (k === "ArrowDown") act.move(1);
        else if (k === "ArrowUp") act.move(-1);
        else if (k.length === 1) act.setQuery(s.query + k);
        else return false;
        return true;
      }
      const move = (d) => (focus === "groups" ? act.setGroup(s.group + d) : act.move(d));
      const jump = (end) => {
        const n = focus === "groups" ? groups().length : items().length;
        const i = end ? n - 1 : 0;
        focus === "groups" ? act.setGroup(i) : act.setSel(i);
      };
      switch (k) {
        case "1":
          return act.setSection("live"), true;
        case "2":
          return act.setSection("movies"), true;
        case "3":
          return act.setSection("series"), true;
        case "j":
        case "ArrowDown":
          return move(1), true;
        case "k":
        case "ArrowUp":
          return move(-1), true;
        case "PageDown":
          return move(10), true;
        case "PageUp":
          return move(-10), true;
        case "g":
        case "Home":
          return jump(false), true;
        case "G":
        case "End":
          return jump(true), true;
        case "h":
        case "ArrowLeft":
          return (focus = "groups"), render(), true;
        case "l":
        case "ArrowRight":
          return (focus = "list"), render(), true;
        case "Tab":
          return (focus = focus === "list" ? "groups" : "list"), render(), true;
        case "Enter":
          if (focus === "groups") return (focus = "list"), render(), true;
          return act.activate(), true;
        case "/":
          return (mode = "search"), (focus = "list"), render(), true;
        case "Escape":
          if (s.query) act.setQuery("");
          else if (s.series) act.back();
          return true;
        case "Backspace":
          return act.back(), true;
        case "i":
          return act.toggleDetails(), true;
        case "u":
          return act.toggleUrl(), true;
        case "a":
          acctSel = s.account;
          mode = "accounts";
          return render(), true;
        case " ":
          return act.pause(), true;
        case "s":
          return act.stop(), true;
        case "q":
          return act.quit(), true;
        default:
          return false;
      }
    };

    subs.push(render);
    return { root, key, resetMode: () => (mode = "normal") };
  })();

  // ------------------------------------------------------------------- mount
  const Demo = {
    active: false,
    focus() {
      Demo.active = true;
      const el = s.view === "tui" ? Tui.root : Gtk.root;
      if (el) el.focus({ preventScroll: true });
      document.getElementById("demo").classList.add("is-active");
    },
  };

  const demo = document.getElementById("demo");
  if (!demo) return;
  Theme.bindDemo(demo);
  const stage = demo.querySelector(".stage");
  stage.append(Gtk.root, Tui.root, Mpv.root);
  const keysBar = document.getElementById("tui-keys");
  const foot = document.getElementById("demo-foot");

  const KEYS = [
    ["j", "↓"], ["k", "↑"], ["h", "←"], ["l", "→"], ["/", "search"], ["Enter", "↵ play"], ["Escape", "esc"],
    [" ", "space"], ["s", "s stop"], ["i", "i info"], ["u", "u url"], ["a", "a accounts"], ["1", "1 live"], ["2", "2 movies"], ["3", "3 series"], ["q", "q quit"],
  ];
  KEYS.forEach(([k, label]) =>
    keysBar.append(
      h("button", { class: "keycap", "aria-label": `Key ${label}`, onclick: () => { Demo.focus(); Tui.key(k); }, text: label }),
    ),
  );

  const GUI_HINT = "Type to search · ↑↓ move · Enter play · Ctrl+P pause · Ctrl+S stop · Ctrl+I details · F5 refresh. Point at the video for its controls; double-click it for fullscreen. (Browsers keep some Ctrl shortcuts, so use the buttons or Alt+1/2/3 for sections.)";
  const TUI_HINT = "Click the terminal, then use your keyboard like in the real thing. The terminal app plays in mpv's own window. On a phone, use the keys below.";

  const tabs = demo.querySelectorAll("[data-view]");
  const syncView = () => {
    tabs.forEach((t) => {
      const on = t.dataset.view === s.view;
      t.classList.toggle("on", on);
      t.setAttribute("aria-selected", String(on));
    });
    Gtk.root.hidden = s.view !== "gui";
    Tui.root.hidden = s.view !== "tui";
    keysBar.hidden = s.view !== "tui";
    foot.textContent = s.view === "gui" ? (touch.matches ? "Tap a channel to play it in the window. Tap the video for its controls." : GUI_HINT) : TUI_HINT;
    demo.dataset.view = s.view;
  };
  tabs.forEach((t) => t.addEventListener("click", () => (Tui.resetMode(), act.setView(t.dataset.view))));
  subs.push(syncView);

  // Keys only reach the demo once the user has clicked into it.
  document.addEventListener("pointerdown", (e) => {
    const inside = !!e.target.closest("#demo");
    Demo.active = inside;
    demo.classList.toggle("is-active", inside);
  });
  document.addEventListener("keydown", (e) => {
    if (!Demo.active || e.defaultPrevented) return;
    if (e.target.closest && e.target.closest("#demo") === null) return;
    const handled = s.view === "tui" ? Tui.key(e.key, e.ctrlKey) : Gtk.key(e);
    if (handled) e.preventDefault();
  });

  // Account picker, theme tabs and the checklist live in the page around it.
  document.querySelectorAll("[data-demo-view]").forEach((b) =>
    b.addEventListener("click", () => {
      act.setView(b.dataset.demoView);
    }),
  );

  // The "now playing" card in the page's hero mirrors the demo.
  const np = document.getElementById("np");
  if (np) {
    const set = (id, text) => (document.getElementById(id).textContent = text);
    subs.push(() => {
      const p = s.playing;
      np.classList.toggle("live", !!p && !p.paused);
      np.classList.toggle("paused", !!p && p.paused);
      if (!p) {
        set("np-state", "Idle");
        set("np-title", "Nothing yet");
        set("np-sub", "Press Enter on a channel in the demo");
        return;
      }
      const it = p.item;
      set("np-state", p.paused ? "Paused" : it.kind === "live" ? "Live" : "Playing");
      set("np-title", it.show ? `${it.show} · ${it.name}` : it.name);
      const where = s.view === "gui" && !s.detached ? "in the window" : "mpv window";
      set("np-sub", [it.kind === "live" ? `#${it.num}` : null, it.group, it.format, where].filter(Boolean).join(" · "));
    });
  }

  emit();
  window.TunerDemo = { focus: Demo.focus, state: s, act };
})();
