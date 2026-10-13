// Riding: speed, distance, ride time (stopped while paused), climbing.

const KNOTS_TO_KMH = 1.852;
const THRESHOLD_M = 3;

// Nothing heard from the GPS yet.
function init() {
  return {
    t: null,          // the latest time seen, epoch ms
    last_fix: null,   // {lat, lon}
    metres: 0,        // distance ridden while not paused
    ride_ms: 0,       // time ridden while not paused
    paused: false,
    speed: 0,         // km/h
    gain: 0,          // metres climbed
    ref_alt: null,    // climbing: the high of this climb; else the lowest since
    climbing: false,
  };
}

function step(state, input, now) {
  const s = state;
  switch (input.tag) {
    case "Location": {
      const ts = input.timestamp;
      const f = input.fix;
      const riding = !s.paused && s.t != null;
      const leg = riding && f != null && s.last_fix != null ? distance(s.last_fix, f) : 0;
      const alt = f?.altitude;
      const climb = s.paused || alt == null ? s : climbing(s, alt);
      return {
        state: {
          t: ts,
          last_fix: f != null ? { lat: f.lat, lon: f.lon } : s.last_fix,
          metres: s.metres + leg,
          ride_ms: s.ride_ms + (riding ? ts - s.t : 0),
          paused: s.paused,
          speed: input.velocity != null ? input.velocity.speed * KNOTS_TO_KMH : s.speed,
          gain: climb.gain,
          ref_alt: climb.ref_alt,
          climbing: climb.climbing,
        },
        wake: null,
      };
    }
    // Pausing banks the time ridden up to the press; resuming restarts the
    // clock from the press.
    case "PauseResume":
      return {
        state: {
          ...s,
          t: s.t == null ? null : now,
          ride_ms: s.ride_ms + (!s.paused && s.t != null ? now - s.t : 0),
          paused: !s.paused,
        },
        wake: null,
      };
    case "Wake":
      return { state: s, wake: null };
    default:
      throw new Error(`riding: unknown input ${input.tag}`);
  }
}

// Climbing: every new high counts; 3 m below it, the climb is over.
// Not climbing: track the low; 3 m above it, count the whole rise.
function climbing(s, alt) {
  const r = s.ref_alt;
  return r == null ? { gain: s.gain, ref_alt: alt, climbing: false }
    : s.climbing && alt > r ? { gain: s.gain + alt - r, ref_alt: alt, climbing: true }
    : s.climbing && alt <= r - THRESHOLD_M ? { gain: s.gain, ref_alt: alt, climbing: false }
    : s.climbing ? { gain: s.gain, ref_alt: r, climbing: true }
    : alt >= r + THRESHOLD_M ? { gain: s.gain + alt - r, ref_alt: alt, climbing: true }
    : { gain: s.gain, ref_alt: Math.min(r, alt), climbing: false };
}

function view(state) {
  return {
    speed_kmh: state.speed,
    avg_speed_kmh: state.ride_ms > 0 ? (state.metres / 1000) / (state.ride_ms / 3600000) : 0,
    distance_km: state.metres / 1000,
    elevation_gain_m: state.gain,
    ride_time_ms: state.ride_ms,
    paused: state.paused,
  };
}
