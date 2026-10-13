// Race: the start sequence, setting the line, and when and where the boat
// would cross it on its current course.
//
// Inputs: LineStbd, LinePort, BumpSeq{seconds, timestamp}, RaceFinish from
// clients; Location{timestamp, fix?, velocity?} and Wake from the host.
// Times are epoch ms on the device clock.

const KNOTS_TO_MPS = 0.514444;

// No sequence running, no line set, nothing heard from the GPS yet.
function init() {
  return {
    phase: { tag: "Active" },               // Active | InSequence{start_time} | Racing{start_time}
    line: { tag: "None" },                  // None | Stbd{stbd} | Port{port} | Both{stbd, port, cross, time}
    location: { lat: 0.0, lon: 0.0 },       // the boat, degrees
    velocity: { speed: 0.0, heading: 0.0 }, // knots, degrees true
  };
}

function step(state, input, now) {
  const next = update(state, input);
  return {
    state: next,
    wake: next.phase.tag === "InSequence" ? next.phase.start_time : null, // the gun
  };
}

function update(state, input) {
  const { phase, line, location: here } = state;
  switch (input.tag) {
    // Set an end of the line where the boat is. Setting the second end makes
    // the line Both; where and when we cross it comes with the next GPS update.
    case "LineStbd":
      return {
        ...state,
        line: line.tag === "None" || line.tag === "Stbd"
          ? { tag: "Stbd", stbd: here }
          : { tag: "Both", stbd: here, port: line.port, cross: line.cross ?? 0, time: line.time ?? 0 },
      };

    case "LinePort":
      return {
        ...state,
        line: line.tag === "None" || line.tag === "Port"
          ? { tag: "Port", port: here }
          : { tag: "Both", stbd: line.stbd, port: here, cross: line.cross ?? 0, time: line.time ?? 0 },
      };

    case "BumpSeq":
      return { ...state, phase: { tag: "InSequence", start_time: gun(phase, input.seconds, input.timestamp) } };

    // Finish the race, or abort the sequence (and with it the gun).
    case "RaceFinish":
      return { ...state, phase: { tag: "Active" } };

    // The gun. Only in a sequence: a stale wake (raced with a RaceFinish) is
    // ignored. The heading reads 0 until the first GPS update after the start.
    case "Wake":
      return phase.tag !== "InSequence" ? state : {
        ...state,
        phase: { tag: "Racing", start_time: phase.start_time },
        velocity: { speed: state.velocity.speed, heading: 0.0 },
      };

    // GPS. Before the start, a fix with a velocity also works out where
    // (cross: % of the way from the port end) and when (time) we would cross
    // a fully set line.
    case "Location": {
      const boat = input.fix ?? here;
      const v = input.velocity;
      const crossing = input.fix != null && v != null && phase.tag !== "Racing" && line.tag === "Both";
      return {
        phase,
        line: crossing ? lineCrossing(line, boat, v, input.timestamp) : line,
        location: boat,
        velocity: v ?? state.velocity,
      };
    }

    default:
      throw new Error(`race: unknown input ${input.tag}`);
  }
}

// The gun after a BumpSeq at `tap`. Not in a sequence: start one with the
// gun `seconds` from the tap. In one: seconds > 0 moves the gun earlier,
// < 0 later, 0 rounds the time left (from the tap) down to a whole minute.
// Never before time 0.
function gun(phase, seconds, tap) {
  const offset = Math.abs(seconds) * 1000;
  const start = phase.start_time; // in a sequence
  return phase.tag !== "InSequence" ? (seconds < 0 ? Math.max(0, tap - offset) : tap + offset)
    : seconds === 0 ? start - Math.max(0, start - tap) % 60000
    : seconds < 0 ? start + offset
    : Math.max(0, start - offset);
}

// Where (cross: % of the way from port to stbd, as Rust's `as u8`) and when
// the boat at `boat`, moving at `v`, reaches the line.
function lineCrossing(line, boat, v, timestamp) {
  const { stbd, port } = line;
  const mps = Math.max(v.speed, 1e-5) * KNOTS_TO_MPS; // never 0
  const length = Math.max(distance(stbd, port), 1.0);

  // Pointing at the line: the starboard end is to the right of the port
  // end, and our course lies between them.
  const toStbd = bearing(boat, stbd);
  const toPort = bearing(boat, port);
  const pointing = angle_diff(toStbd, toPort) > 0
    && angle_diff(toStbd, v.heading) > 0
    && angle_diff(v.heading, toPort) > 0;
  const x = pointing ? course_crosses_line(boat, v.heading, stbd, port) : null;

  // along: 0 at the port end .. 1 at stbd; seconds: until we're there.
  // Not pointing at it (or no crossing within 200 km): the nearer end.
  const toS = distance(boat, stbd);
  const toP = distance(boat, port);
  const along = x != null ? distance(port, x) / length : toS < toP ? 1.0 : 0.0;
  const seconds = (x != null ? distance(boat, x) : Math.min(toS, toP)) / mps;

  const pct = along * 100;
  return {
    tag: "Both", stbd, port,
    cross: pct >= 255 ? 255 : pct >= 0 ? Math.trunc(pct) : 0,
    time: Math.max(0, timestamp + Math.trunc(seconds * 1000)),
  };
}

// What clients see of a race (RaceView). speed and heading are f32 on the
// wire: the view schema narrows them, not the code.
function view(state) {
  // The line as far as it has been set; its ends and length stay private.
  const line = state.line.tag === "Both"
    ? { tag: "Both", cross: state.line.cross, time: state.line.time }
    : { tag: state.line.tag };
  const { phase } = state;
  return {
    speed: state.velocity.speed, // knots
    phase:
      phase.tag === "Racing"
        // the line is no longer shown; the course is
        ? { tag: "Racing", start_time: phase.start_time, heading: state.velocity.heading }
        : phase.tag === "InSequence"
          ? { tag: "InSequence", start_time: phase.start_time, line }
          : { tag: "Active", line },
  };
}
