// Vendée: next waypoint (passed within 10 nm), distance to it and to the
// finish along the course, the 24 h run along the track, VMG to the next.

const NM = 1852;
const RADIUS_NM = 10;
const DAY_MS = 86400000;
const SAMPLE_MS = 900000; // a 24 h run sample every 15 min
const SAMPLES = 100;      // 25 h of them

const COURSE = [
  { name: "Finisterre", lat: 43.0, lon: -10.0 },
  { name: "Canaries", lat: 28.0, lon: -19.0 },
  { name: "Equator Out", lat: 0.0, lon: -27.0 },
  { name: "Good Hope", lat: -42.0, lon: 18.5 },
  { name: "Leeuwin", lat: -45.0, lon: 115.0 },
  { name: "Campbell", lat: -50.0, lon: 170.0 },
  { name: "Point Nemo", lat: -49.0, lon: -123.0 },
  { name: "Cape Horn", lat: -57.0, lon: -67.3 },
  { name: "Equator Home", lat: 0.0, lon: -30.0 },
  { name: "Finish", lat: 46.47, lon: -1.79 },
];
const LAST = COURSE.length - 1;

// Heading for Finisterre, no position yet.
function init() {
  return {
    next: 0,          // index into the course: Finisterre
    pos: null,        // {lat, lon}
    t: null,          // the latest fix's time
    sailed_nm: 0,     // along the track since the first fix
    samples: [],      // {t, nm}: sailed_nm every 15 min, the last 100 (25 h)
    run_nm: 0,        // the 24 h run
    to_next: null,    // nm
    to_finish: null,  // nm
    vmg: null,        // knots
  };
}

function step(state, input, now) {
  switch (input.tag) {
    case "Location":
      return { state: input.fix == null ? state : location(state, input), wake: null };
    case "Wake":
      return { state, wake: null };
    default:
      throw new Error(`vendee: unknown input ${input.tag}`);
  }
}

function location(s, input) {
  const ts = input.timestamp;
  const p = { lat: input.fix.lat, lon: input.fix.lon };
  const sailed = s.sailed_nm + (s.pos == null ? 0 : distance(s.pos, p) / NM);

  // Passed the target? (At most one per fix; the finish stays.)
  const passed = s.next < LAST && distance(p, COURSE[s.next]) / NM <= RADIUS_NM;
  const n = passed ? s.next + 1 : s.next;
  const wp = COURSE[n];
  const toNext = distance(p, wp) / NM;
  const samples = s.samples.length === 0 || ts - last(s.samples).t >= SAMPLE_MS
    ? push_last(s.samples, { t: ts, nm: sailed }, SAMPLES)
    : s.samples;
  const v = input.velocity;
  return {
    next: n,
    pos: p,
    t: ts,
    sailed_nm: sailed,
    samples,
    run_nm: sailed - sailedAt(samples, ts - DAY_MS, { t: ts, nm: sailed }),
    to_next: toNext,
    to_finish: toNext + track_distance(COURSE.slice(n)) / NM,
    vmg: v == null ? null : v.speed * Math.cos(angle_diff(v.heading, bearing(p, wp)) * Math.PI / 180),
  };
}

// The distance sailed at time `cut`, interpolated between the samples
// either side of it (`now` is the latest); 0 before the first sample, so a
// run with less than 24 h of samples is all that was sailed.
function sailedAt(samples, cut, now) {
  const i = samples.findLastIndex((x) => x.t <= cut);
  const a = samples[i]; // none if i < 0
  const b = samples[i + 1] ?? now;
  return i < 0 ? 0
    : b.t === a.t ? a.nm
    : a.nm + (b.nm - a.nm) * (cut - a.t) / (b.t - a.t);
}

function view(state) {
  return {
    next_waypoint: COURSE[state.next].name,
    to_next_nm: state.to_next,
    to_finish_nm: state.to_finish,
    run_24h_nm: state.run_nm,
    vmg_kn: state.vmg,
  };
}
