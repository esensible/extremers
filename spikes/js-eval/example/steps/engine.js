const IDLE_MS = 10000;

function init() {
  return { count: 0, recent: [], idle: false };
}

function step(state, input, now) {
  const next = update(state, input, now);
  const latest = last(next.recent);
  return { state: next, wake: next.idle || latest == null ? null : latest + IDLE_MS };
}

function update(state, input, now) {
  switch (input.tag) {
    case "Step":
      return { count: state.count + 1, recent: push_last(state.recent, now, 10), idle: false };
    case "Reset":
      return { count: 0, recent: [], idle: false };
    case "Wake": {
      const latest = last(state.recent);
      return { ...state, idle: latest != null && now >= latest + IDLE_MS };
    }
    case "Location":
      return state;
    default:
      throw new Error(`steps: unknown input ${input.tag}`);
  }
}

function view(state) {
  const { recent } = state;
  return {
    count: state.count,
    idle: state.idle,
    cadence_spm: recent.length < 2 ? null : 60000 / mean(pairs(recent), ([a, b]) => b - a),
  };
}
