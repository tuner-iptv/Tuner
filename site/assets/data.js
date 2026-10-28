// Mock data for the demo. Nothing here is a real channel or title; the live
// list mirrors the "mock" provider used for the screenshots in docs/images.
(() => {
  const live = (n, name, group, extra = {}) => ({
    kind: "live",
    id: `live-${n}`,
    num: n - 1000,
    logo: String(n),
    name,
    group,
    catchup: extra.catchup ?? 0,
    epg: `demo.${n}`,
    format: extra.format ?? "MPEG-TS",
    hue: (n * 47) % 360,
    offline: !!extra.offline,
    path: `live/demo/demo/${n}.${extra.format === "HLS" ? "m3u8" : "ts"}`,
  });

  const channels = [
    live(1001, "Demo News HD", "News", { catchup: 3 }),
    live(1002, "World Bulletin", "News", { catchup: 3 }),
    live(1003, "Café Istanbul", "News", { format: "HLS" }),
    live(1004, "Local News", "News"),
    live(1005, "Night Report", "News", { catchup: 7 }),
    live(1006, "Demo Sports HD", "Sports", { catchup: 3 }),
    live(1007, "Stadium", "Sports"),
    live(1008, "Racing Lab", "Sports", { format: "HLS" }),
    live(1009, "Court Side", "Sports"),
    live(1010, "Weekend Sports", "Sports", { catchup: 1 }),
    live(1011, "Nature & Science", "Nature & Science", { catchup: 3 }),
    live(1012, "Ocean Lab", "Nature & Science"),
    live(1013, "Space Window", "Nature & Science", { format: "HLS" }),
    live(1014, "Mountain Trails", "Nature & Science"),
    live(1015, "Wildlife", "Nature & Science", { catchup: 3 }),
    live(1016, "Demo Entertainment", "Entertainment"),
    live(1017, "Music Lab", "Entertainment"),
    live(1018, "Retro Patterns", "Entertainment", { format: "HLS" }),
    live(1019, "Cinema Preview", "Entertainment", { catchup: 3 }),
    live(1020, "Offline Channel (intentional)", "Entertainment", { offline: true }),
  ];

  // What `tuner examples/sample.m3u` loads (examples/sample.m3u in the repo).
  const sample = [
    {
      kind: "live",
      id: "m3u-1",
      num: 1,
      logo: "",
      name: "Big Buck Bunny (HLS)",
      group: "Test Streams",
      catchup: 0,
      epg: "bbb",
      format: "HLS",
      hue: 24,
      path: "x36xhzz/x36xhzz.m3u8",
      host: "test-streams.mux.dev",
    },
    {
      kind: "live",
      id: "m3u-2",
      num: 2,
      logo: "",
      name: "Tears of Steel (HLS)",
      group: "Test Streams",
      catchup: 0,
      epg: "tos",
      format: "HLS",
      hue: 200,
      path: "k8s/features/stable/video/tears-of-steel/tears-of-steel.ism/.m3u8",
      host: "demo.unified-streaming.com",
    },
    {
      kind: "live",
      id: "m3u-3",
      num: 3,
      logo: "",
      name: "Radio Paradise (AAC)",
      group: "Audio",
      catchup: 0,
      epg: "",
      format: "AAC",
      hue: 300,
      path: "aac-128",
      host: "stream.radioparadise.com",
    },
  ];

  const movie = (i, name, category, year, rating, plot, hue) => ({
    kind: "movie",
    id: `movie-${i}`,
    logo: "",
    name,
    group: category,
    year,
    rating,
    plot,
    hue,
    format: "MKV",
    path: `movie/demo/demo/${2000 + i}.mkv`,
  });

  const movies = [
    movie(1, "The Last Lighthouse", "Drama", 2021, 7.8, "A keeper on a forgotten rock must decide whether to light the lamp one final time.", 210),
    movie(2, "Midnight Courier", "Action", 2023, 6.9, "One night, forty-two deliveries and a package nobody was supposed to open.", 12),
    movie(3, "Salt & Stone", "Documentary", 2019, 8.3, "Three generations of salt harvesters on the edge of a vanishing coast.", 40),
    movie(4, "Paper Moons", "Comedy", 2022, 7.1, "A failing puppet theatre bets everything on one very strange season.", 320),
    movie(5, "Hollow Ridge", "Action", 2020, 6.4, "A rescue team crosses a collapsing mountain pass before the storm closes it.", 150),
    movie(6, "Quiet Frequencies", "Drama", 2024, 8.0, "A radio operator starts hearing a voice that should not exist.", 260),
    movie(7, "The Great Bake-Off Heist", "Comedy", 2018, 6.6, "Four bakers, one prize recipe, no plan whatsoever.", 350),
    movie(8, "Deep Field", "Documentary", 2022, 8.6, "Astronomers spend a year chasing a single faint signal across the sky.", 230),
    movie(9, "Neon Rain", "Action", 2017, 7.2, "A courier on a flooded city's rooftops outruns the people she works for.", 290),
    movie(10, "Letters from Aldermoor", "Drama", 2016, 7.5, "A village postwoman reads the mail nobody else would dare to deliver.", 100),
    movie(11, "Tiny Giants", "Documentary", 2021, 7.9, "The hidden lives of the smallest animals in the largest forests.", 130),
    movie(12, "Uncle Gus Takes a Cruise", "Comedy", 2015, 6.2, "A retired plumber, a cruise ship, and an unfortunate misunderstanding.", 60),
  ];

  const episodeTitles = [
    "Pilot", "The Offer", "Static", "Low Tide", "Second Chances", "Open Water",
    "Fault Lines", "The Long Way Round", "Homecoming", "Last Light", "Crossing", "Signal Fire",
  ];

  const show = (i, name, category, year, rating, plot, hue, seasons, perSeason) => {
    const id = `series-${i}`;
    const eps = [];
    for (let s = 1; s <= seasons; s++) {
      for (let e = 1; e <= perSeason; e++) {
        const title = episodeTitles[(i * 3 + s * 5 + e) % episodeTitles.length];
        eps.push({
          kind: "episode",
          id: `${id}-s${s}e${e}`,
          logo: "",
          name: `S${s}E${e} · ${title}`,
          group: `Season ${s}`,
          season: s,
          year,
          rating,
          plot,
          hue: (hue + s * 20 + e * 6) % 360,
          format: "MKV",
          path: `series/demo/demo/${3000 + i * 100 + s * 10 + e}.mkv`,
          show: name,
        });
      }
    }
    return { kind: "series", id, logo: "", name, group: category, year, rating, plot, hue, episodes: eps, seasons };
  };

  const series = [
    show(1, "Harbor Lights", "Drama", 2022, 8.4, "Small-town harbour, big secrets. Season two picks up the morning after the storm.", 200, 2, 4),
    show(2, "Orbit Hotel", "Comedy", 2023, 7.7, "The staff of a very remote space hotel try to look busy when inspectors arrive.", 280, 2, 4),
    show(3, "Cold Case Kitchen", "Crime", 2021, 7.3, "A chef-turned-detective solves cases by recreating the last meal.", 20, 3, 3),
    show(4, "The Understudies", "Comedy", 2020, 7.0, "Backstage at a theatre where the understudies are always better.", 330, 2, 3),
    show(5, "Glacier", "Documentary", 2024, 8.7, "A multi-year look at one glacier, told season by season.", 190, 2, 4),
    show(6, "Night Shift Radio", "Drama", 2019, 7.9, "Every call to the late-night station changes someone's life, including the host's.", 250, 3, 3),
  ];

  const xtream = (name, o) => ({
    name,
    type: "Xtream Codes",
    server: "localhost:8080",
    expires: "2027-10-27",
    vod: true,
    ...o,
  });

  const accounts = [
    xtream("mock", { limit: 8, active: 0, mode: "confirm", note: "Plenty of connections" }),
    xtream("home", { limit: 1, active: 1, mode: "confirm", note: "Limit 1; another device is already watching" }),
    xtream("backup", { limit: 2, active: 2, mode: "block", note: "Full, and set to never play" }),
    {
      name: "sample.m3u",
      type: "M3U playlist",
      server: "local file",
      expires: "",
      vod: false,
      limit: 1,
      active: 0,
      mode: "confirm",
      note: "examples/sample.m3u (no movies or series)",
      m3u: true,
    },
  ];

  window.TunerData = { channels, sample, movies, series, accounts };
})();
