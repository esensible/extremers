// Running: elapsed time, distance, ~30 s pace, kilometre splits, laps.

const PACE_WINDOW_MS = 30000;
const SAMPLES = 90; // fixes kept for the pace: 30 s at 1 Hz, with room
const SPLITS = 5;

// Nothing heard from the GPS yet.
function init() {
  return {
    start: null,      // epoch ms of the first fix
    t: null,          // the latest time seen (a fix or a Lap press)
    last_fix: null,   // {lat, lon} of the latest fix
    last_fix_t: null, // its timestamp
    metres: 0,        // track distance
    recent: [],       // {t, m}: the last fixes, cumulative metres, for the pace
    pace: null,       // s/km over ~30 s
    km_done: 0,       // whole kilometres completed
    km_start: null,   // when the current kilometre began
    splits: [],       // ms each, the last 5
    lap_start: null,
    last_lap: null,
  };
}

function step(state, input, now) {
  switch (input.tag) {
    case "Location":
      return { state: input.fix == null ? state : location(state, input), wake: null };
    case "Lap":
      return {
        state: state.start == null ? state : { ...state, t: now, lap_start: now, last_lap: now - state.lap_start },
        wake: null,
      };
    case "Wake":
      return { state, wake: null };
    default:
      throw new Error(`running: unknown input ${input.tag}`);
  }
}

function location(s, input) {
  const ts = input.timestamp;
  const isFirst = s.start == null;
  const leg = isFirst ? 0 : distance(s.last_fix, input.fix);
  const metres = s.metres + leg;
  const done = Math.trunc(metres / 1000);
  // A kilometre ended inside this leg: when, interpolated along it.
  const crossed = !isFirst && done > s.km_done && leg > 0;
  const at = crossed
    ? s.last_fix_t + Math.trunc((ts - s.last_fix_t) * (done * 1000 - s.metres) / leg)
    : 0;
  const recent = push_last(s.recent, { t: ts, m: metres }, SAMPLES);
  const oldest = recent.find((r) => r.t >= ts - PACE_WINDOW_MS);
  const moved = metres - oldest.m;
  return {
    start: isFirst ? ts : s.start,
    t: ts,
    last_fix: { lat: input.fix.lat, lon: input.fix.lon },
    last_fix_t: ts,
    metres,
    recent,
    pace: ts - oldest.t >= 5000 && moved > 1 ? (ts - oldest.t) / moved : null,
    km_done: done,
    km_start: isFirst ? ts : crossed ? at : s.km_start,
    splits: crossed ? push_last(s.splits, at - s.km_start, SPLITS) : s.splits,
    lap_start: isFirst ? ts : s.lap_start,
    last_lap: s.last_lap,
  };
}

function view(state) {
  return {
    elapsed_ms: state.start == null ? 0 : state.t - state.start,
    distance_m: state.metres,
    pace_s_per_km: state.pace,
    splits_ms: state.splits,
    lap_ms: state.start == null ? 0 : state.t - state.lap_start,
    last_lap_ms: state.last_lap,
  };
}
