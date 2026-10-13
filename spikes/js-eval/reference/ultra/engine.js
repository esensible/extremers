// Ultra: course distance (GPS, reset to the station's at each check-in),
// and the pace of the last ~10 minutes for the ETA (worked out in the view).

const PACE_WINDOW_MS = 600000;
const SAMPLE_MS = 10000;
const SAMPLES = 80; // 13 min of 10 s samples

const STATIONS = [
  { name: "Ridge Hut", km: 12.0, cutoff_ms: 9000000 },       // 2 h 30
  { name: "River Crossing", km: 25.5, cutoff_ms: 18000000 }, // 5 h
  { name: "Summit Camp", km: 41.0, cutoff_ms: 30600000 },    // 8 h 30
  { name: "Finish", km: 56.0, cutoff_ms: 43200000 },         // 12 h
];
const LAST = STATIONS.length - 1;

// On the start line, no GPS yet.
function init() {
  return {
    start: null,    // the first fix's time: the race start
    t: null,        // the latest fix's time
    last_fix: null, // {lat, lon}
    track_m: 0,     // GPS track since the start
    checkin_m: 0,   // track_m at the last check-in
    base_km: 0,     // the course distance of the last station checked in at
    next: 0,        // index of the next station: Ridge Hut
    samples: [],    // {t, m}: track_m every 10 s, for the recent pace
    pace: null,     // ms per metre over the last ~10 min
  };
}

function step(state, input, now) {
  const s = state;
  switch (input.tag) {
    case "Location":
      return { state: input.fix == null ? s : location(s, input), wake: null };
    // Checked in at the next station: we're at its distance; on to the one after.
    case "CheckIn":
      return {
        state: {
          ...s,
          checkin_m: s.track_m,
          base_km: STATIONS[s.next].km,
          next: Math.min(s.next + 1, LAST),
        },
        wake: null,
      };
    case "Wake":
      return { state: s, wake: null };
    default:
      throw new Error(`ultra: unknown input ${input.tag}`);
  }
}

function location(s, input) {
  const ts = input.timestamp;
  const p = { lat: input.fix.lat, lon: input.fix.lon };
  const track = s.track_m + (s.last_fix == null ? 0 : distance(s.last_fix, p));
  const samples = s.samples.length === 0 || ts - s.samples.at(-1).t >= SAMPLE_MS
    ? push_last(s.samples, { t: ts, m: track }, SAMPLES)
    : s.samples;
  const oldest = samples.find((x) => x.t >= ts - PACE_WINDOW_MS);
  const span = ts - oldest.t;
  const moved = track - oldest.m;
  return {
    ...s,
    start: s.start ?? ts,
    t: ts,
    last_fix: p,
    track_m: track,
    samples,
    pace: span >= 60000 && moved >= 50 ? span / moved : null,
  };
}

function view(state) {
  const st = STATIONS[state.next];
  const covered = state.base_km + (state.track_m - state.checkin_m) / 1000;
  const left = Math.max(0, st.km - covered);
  const eta = state.pace == null || state.t == null ? null : state.t + Math.trunc(left * 1000 * state.pace);
  return {
    next_station: st.name,
    distance_km: covered,
    to_next_km: left,
    eta_ms: eta,
    in_hand_ms: eta == null ? null : state.start + st.cutoff_ms - eta,
  };
}
