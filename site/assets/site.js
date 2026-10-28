// Page behaviour around the demo: nav, screenshot tour, copy buttons, light/dark
// switch, and the latest release from GitHub (the page works without it).
(() => {
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => [...r.querySelectorAll(s)];
  const Theme = window.TunerTheme;
  const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;

  // mobile menu
  const menuBtn = $("#menu-btn");
  const links = $("#nav-links");
  menuBtn.addEventListener("click", () => {
    const open = links.classList.toggle("open");
    menuBtn.setAttribute("aria-expanded", String(open));
  });
  links.addEventListener("click", (e) => {
    if (e.target.closest("a")) {
      links.classList.remove("open");
      menuBtn.setAttribute("aria-expanded", "false");
    }
  });

  // "Try it" scrolls to the demo and hands it the keyboard
  $$("[data-try]").forEach((a) =>
    a.addEventListener("click", () => setTimeout(() => window.TunerDemo && window.TunerDemo.focus(), 450)),
  );

  // page light / dark switch (the demo has its own theme picker)
  $("#mode-btn").addEventListener("click", () => Theme.setMode(Theme.mode === "dark" ? "light" : "dark"));

  // screenshot tour: the active item's bar fills, then the next screen shows
  const tour = $("#tour-box");
  const items = $$(".tour-item", tour);
  const shots = $$(".tour-shot", tour);
  const pauseBtn = $("#tour-pause");
  let current = 0;
  const show = (i) => {
    current = (i + items.length) % items.length;
    items.forEach((el, j) => {
      el.classList.toggle("on", j === current);
      el.setAttribute("aria-selected", String(j === current));
    });
    shots.forEach((el, j) => el.classList.toggle("on", j === current));
  };
  const setPaused = (p) => {
    tour.classList.toggle("paused", p);
    pauseBtn.setAttribute("aria-pressed", String(p));
    pauseBtn.textContent = p ? "Play" : "Pause";
  };
  items.forEach((el, j) => el.addEventListener("click", () => show(j)));
  tour.addEventListener("animationend", (e) => {
    if (e.animationName === "tour-fill" && !reduceMotion) show(current + 1);
  });
  $(".tour-list", tour).addEventListener("keydown", (e) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    show(current + (e.key === "ArrowDown" ? 1 : -1));
    items[current].focus();
  });
  pauseBtn.addEventListener("click", () => setPaused(!tour.classList.contains("paused")));
  setPaused(reduceMotion);

  // copy buttons
  $$(".cmd").forEach((box) => {
    const btn = $(".copy", box);
    btn.addEventListener("click", async () => {
      const code = $("code", box);
      try {
        await navigator.clipboard.writeText(code.innerText);
        btn.textContent = "Copied";
      } catch (_) {
        const r = document.createRange();
        r.selectNodeContents(code);
        const sel = getSelection();
        sel.removeAllRanges();
        sel.addRange(r);
        btn.textContent = "Ctrl+C";
      }
      setTimeout(() => (btn.textContent = "Copy"), 1800);
    });
  });

  // latest release (optional nicety; falls back to the static text)
  fetch("https://api.github.com/repos/tuner-iptv/Tuner/releases/latest", { headers: { Accept: "application/vnd.github+json" } })
    .then((r) => (r.ok ? r.json() : Promise.reject()))
    .then((rel) => {
      if (!rel.tag_name) return;
      $("#version-text").textContent = `${rel.tag_name} · Open-source IPTV player`;
      $("#install-version").textContent = rel.tag_name;
      if (rel.published_at) {
        const day = new Date(rel.published_at).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
        $("#rel-meta").textContent = `Released ${day} ·`;
      }
    })
    .catch(() => {});

  // fade cards in as they scroll into view
  if ("IntersectionObserver" in window && !reduceMotion) {
    const io = new IntersectionObserver(
      (entries) =>
        entries.forEach((e) => {
          if (e.isIntersecting) {
            e.target.classList.add("in");
            io.unobserve(e.target);
          }
        }),
      { threshold: 0.12 },
    );
    $$(".card").forEach((el) => {
      el.classList.add("reveal");
      io.observe(el);
    });
  }
})();
