// Tune: speed, and how speed and heading differ from their 30 s average.
//
// Only GPS velocities matter. Never asks for a wake; takes no client inputs.

const WINDOW_MS = 30000;
const HISTORY = 32; // samples kept: more than the window at the GPS's 1 Hz

// Nothing heard from the GPS yet.
function init() {
  return {
    speed: 0.0,       // knots, the latest
    speed_dev: 0.0,   // latest minus the 30 s average, knots
    heading_dev: 0.0, // latest minus the 30 s average, degrees, -180..180

    // private: never in the view
    history: [],          // the last HISTORY samples, oldest first: {speed, heading, ts}
    last_timestamp: null, // of the newest sample
  };
}

function step(state, input, now) {
  switch (input.tag) {
    case "Wake":
      return { state, wake: null }; // not asked for; nothing to do
    case "Location": {
      // A velocity is needed; repeated and out-of-order samples are ignored.
      const v = input.velocity;
      const prev = state.last_timestamp;
      const fresh = v != null && (prev == null || input.timestamp > prev);
      return { state: fresh ? sample(state, v, input.timestamp) : state, wake: null };
    }
    default:
      throw new Error(`tune: unknown input ${input.tag}`);
  }
}

// A new sample: velocity `v` at time `ts`.
function sample(state, v, ts) {
  const history = push_last(state.history, { speed: v.speed, heading: v.heading, ts }, HISTORY);
  const isFirst = state.last_timestamp == null; // nothing to average yet

  // Time-weighted averages over the last 30 s, newest sample first: each
  // sample counts for the seconds until the one after it, clipped to the
  // window (so the first sample older than the window counts for the part
  // inside it, and older ones for nothing).
  const windowStart = Math.max(0, ts - WINDOW_MS);
  const newestFirst = history.toReversed();
  const weighted = newestFirst.map((s, i) => {
    const until = i === 0 ? ts : newestFirst[i - 1].ts;
    return {
      speed: s.speed,
      heading: s.heading,
      seconds: Math.max(0, until - Math.max(s.ts, windowStart)) / 1000,
    };
  });
  const total = sum(weighted, (w) => w.seconds);
  const meanSpeed = total > 0 ? sum(weighted, (w) => w.speed * w.seconds) / total : v.speed;
  const avgHeading = mean_heading(weighted.map((w) => w.heading), weighted.map((w) => w.seconds));

  return {
    speed: v.speed,
    speed_dev: isFirst ? 0.0 : v.speed - meanSpeed,
    heading_dev: isFirst ? 0.0 : angle_diff(v.heading, avgHeading),
    history,
    last_timestamp: ts,
  };
}

// What clients see of tuning (TuneView): three f32s on the wire (the view
// schema narrows them). The history stays private.
function view(state) {
  return {
    speed: state.speed,
    speed_dev: state.speed_dev,
    heading_dev: state.heading_dev,
  };
}
