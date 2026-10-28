// Theming. The page has a light/dark switch (Tokyo Night / Tokyo Night Day).
// The demo is themed separately, with the GTK app's built-in palettes
// (themes-data.js), like the real window: picking a demo theme never changes
// the page around it.
(() => {
  const themes = window.TUNER_THEMES;
  const MODE_KEY = "tuner-site-mode";
  const PAGE = { dark: "tokyo-night", light: "tokyo-night-day" };
  const listeners = [];

  const find = (name) => themes.find((t) => t.name === name) || themes[0];

  const luminance = (hex) => {
    const n = parseInt(hex.slice(1), 16);
    const [r, g, b] = [n >> 16, (n >> 8) & 255, n & 255].map((v) => {
      v /= 255;
      return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };

  // Set a palette as CSS variables on one element (the page root or the demo).
  const paint = (el, t) => {
    const s = el.style;
    s.setProperty("--bg", t.bg);
    s.setProperty("--bg-dark", t.bgDark);
    s.setProperty("--bg-darker", t.bgDarker);
    s.setProperty("--bg-light", t.bgLight);
    s.setProperty("--fg", t.fg);
    s.setProperty("--fg-dim", t.fgDim);
    s.setProperty("--fg-bright", t.fgBright);
    s.setProperty("--accent", t.accent);
    s.setProperty("--sel", t.sel);
    s.setProperty("--muted", t.muted);
    s.setProperty("--warn", t.warn);
    s.setProperty("--err", t.err);
    // Text on top of the accent colour: whichever of the two reads better.
    s.setProperty("--on-accent", luminance(t.accent) > 0.35 ? "#101018" : "#ffffff");
    s.colorScheme = t.dark ? "dark" : "light";
  };

  let mode = "dark";
  let demoEl = null;
  let demoTheme = find(PAGE.dark);
  let demoChosen = false; // until the visitor picks one, the demo follows the page

  const setDemo = (name, { chosen = true } = {}) => {
    demoTheme = find(name);
    if (chosen) demoChosen = true;
    if (demoEl) paint(demoEl, demoTheme);
    listeners.forEach((fn) => fn(demoTheme));
  };

  const setMode = (m, { save = true } = {}) => {
    mode = m === "light" ? "light" : "dark";
    paint(document.documentElement, find(PAGE[mode]));
    document.documentElement.dataset.mode = mode;
    if (!demoChosen) setDemo(PAGE[mode], { chosen: false });
    if (save) {
      try {
        localStorage.setItem(MODE_KEY, mode);
      } catch (_) {
        /* private mode: the choice just won't be remembered */
      }
    }
  };

  let saved = null;
  try {
    saved = localStorage.getItem(MODE_KEY);
  } catch (_) {
    /* ignore */
  }
  setMode(saved || (matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark"), { save: false });

  window.TunerTheme = {
    all: themes,
    get mode() {
      return mode;
    },
    get demo() {
      return demoTheme;
    },
    setMode,
    setDemo,
    bindDemo(el) {
      demoEl = el;
      paint(el, demoTheme);
    },
    onDemoChange: (fn) => listeners.push(fn),
  };
})();
