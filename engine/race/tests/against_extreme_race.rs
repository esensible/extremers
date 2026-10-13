//! Race on the VM (engine.expr, with the sailing builtins) driven through
//! scripted races beside extreme-race (the Rust engine the firmware runs as
//! WASM), and compared after every evaluation:
//! - the **view** (what clients receive) against extreme-race's `RaceView`:
//!   the phase and its gun, the line as which ends are set, the crossing
//!   time, the speed, and the heading while racing;
//! - the **state** on what it holds beyond the view: the line's ends, the
//!   crossing time, and the velocity;
//! - the timer.
//!
//! Both get the same history, as the firmware's host delivers it:
//! - a **GPS update** `{fix?, velocity?}` at time `t`: old
//!   `on_location({t, fix, velocity})`; new: event `{gps: {fix?, velocity?}}`
//!   (the fix also appended to `gps`), `timestamp = t`;
//! - a **client event**: old `on_event`; new: that event's variant;
//! - the **timer**: whenever the engine has one due (old `next_timer`, new
//!   the output's `wake`; the test checks they're equal after every
//!   evaluation), the host delivers it at exactly that time, before
//!   any later input; or at once, if the evaluation that set it put it in
//!   the past. Old `on_timer`; new: event `{wake: {}}`.
//!
//! **Sentinels.** The reference holds placeholders where the new state
//! holds absence, and the test asserts the absence there:
//! - a line end set before any fix is (0, 0) in the reference; here it is
//!   absent (a tap with no fix leaves the line as it was);
//! - a line's crossing is (0 %, time 0) from when its second end is set
//!   until the next GPS update with a fix and a velocity; here it is absent;
//! - the velocity is (0, 0) until the first GPS update with one, and the
//!   reference zeroes the heading at the gun; here it is absent until the
//!   first, and kept at the gun: the view's heading while racing is then
//!   the last one delivered, where the reference's is 0.
//!
//! The view has no crossing percentage (no client shows it), so the
//! reference's `cross` isn't compared.

use compiler::{RunResult, compile, run};
use extreme_race::{LineView, Phase, Race, RaceEvent};
use extreme_sdk::{Engine, Fix, Location, Velocity};
use serde_json::{Map, Value as Json, json};

const PI: f64 = core::f64::consts::PI;
const T0: u64 = 1_790_000_000_000;

/// The one accepted difference in the crossing time, ms (engine/PROFILE.md
/// §4): positions cross the builtin boundary in degrees, so a time
/// truncated to whole ms may come out 1 ms off extreme-race's. Every
/// evaluation that uses it is listed in the report.
const TIME_TOLERANCE_MS: i64 = 1;

#[derive(Clone, Copy, Debug)]
enum Input {
    Gps { fix: Option<(f64, f64)>, velocity: Option<(f64, f64)> },
    Wake,
    Event(RaceEvent),
}

/// The new engine: the compiled program, the state, the view and the wake
/// from its last output, and the track.
struct Vm {
    program: Vec<u8>,
    iface: Json,
    state: Json,
    view: Json,
    wake: Option<u64>,
    gps: Vec<Json>,
}

/// An output's `wake` as the host reads it: absent is no timer. (A nil
/// field is absent: `compiler::run` leaves it out of the record.)
fn wake_of(output: &Json) -> Option<u64> {
    output.get("wake").map(|w| u64::try_from(w.as_i64().unwrap()).unwrap())
}

fn event_json(input: &Input, now: u64) -> Json {
    match *input {
        Input::Gps { fix, velocity } => {
            let mut g = json!({});
            if let Some((lat, lon)) = fix {
                g["fix"] = json!({"t": now, "lat": lat, "lon": lon});
            }
            if let Some((speed, heading)) = velocity {
                g["velocity"] = json!({"speed": speed, "heading": heading});
            }
            json!({"gps": g})
        }
        Input::Wake => json!({"wake": {}}),
        Input::Event(e) => match e {
            RaceEvent::LineStbd => json!({"line_stbd": {}}),
            RaceEvent::LinePort => json!({"line_port": {}}),
            RaceEvent::BumpSeq { seconds, timestamp } => json!({"bump_seq": {"seconds": seconds, "tapped_at": timestamp}}),
            RaceEvent::RaceFinish => json!({"race_finish": {}}),
        },
    }
}

impl Vm {
    fn new() -> Self {
        let iface = engine_race::interface();
        let program = compile(engine_race::PROGRAM, &iface, &Default::default()).unwrap_or_else(|d| panic!("{d:#?}")).bytes;
        Vm { program, iface, state: Json::Null, view: Json::Null, wake: None, gps: Vec::new() }
    }

    /// One evaluation, as the host does it: the program over the event; its
    /// output's `state` becomes the state, its `view` goes to clients, its
    /// `wake` sets the timer. Returns the new state.
    fn eval(&mut self, now: u64, input: &Input) -> Json {
        if let Input::Gps { fix: Some((lat, lon)), .. } = *input {
            self.gps.push(json!({"t": now, "lat": lat, "lon": lon}));
        }
        let mut inputs = Map::new();
        inputs.insert("state".into(), self.state.clone());
        inputs.insert("event".into(), event_json(input, now));
        inputs.insert("timestamp".into(), json!(now));
        inputs.insert("gps".into(), Json::Array(self.gps.clone()));
        match run(&self.program, &self.iface, &inputs, &sailing::BUILTINS).unwrap() {
            RunResult::Ok { value, .. } => self.accept(&plain(&value)),
            RunResult::Failed { code, span } => panic!("at {now} ({input:?}): {code} at {span:?}\n{}", self.state),
        }
        self.state.clone()
    }

    /// A completed evaluation's output, as the host takes it (PROFILE.md
    /// §3): `{state, view, wake?}` and nothing else.
    fn accept(&mut self, output: &Json) {
        let o = output.as_object().unwrap();
        assert!(o.keys().all(|k| k == "state" || k == "view" || k == "wake"), "the output is {{state, view, wake?}}: {output}");
        assert!(o.get("view").is_some_and(Json::is_object), "the output has a view: {output}");
        self.state = o["state"].clone();
        self.view = o["view"].clone();
        self.wake = wake_of(output);
    }
}

/// `compiler::run`'s tagged output as plain JSON (the next evaluation's
/// `state`).
fn plain(v: &Json) -> Json {
    match v["kind"].as_str().unwrap() {
        "record" => Json::Object(v["fields"].as_object().unwrap().iter().map(|(k, f)| (k.clone(), plain(f))).collect()),
        "int" => json!(v["int"].as_str().unwrap().parse::<i64>().unwrap()),
        "float" => json!(f64::from_bits(u64::from_str_radix(v["bits"].as_str().unwrap(), 16).unwrap())),
        "string" => v["string"].clone(),
        "bool" => v["bool"].clone(),
        "nil" => Json::Null,
        k => panic!("unexpected {k}"),
    }
}

/// The old engine, called as the firmware's host calls it.
fn old_eval(race: &mut Race, now: u64, input: &Input) {
    match *input {
        Input::Gps { fix, velocity } => {
            race.on_location(&Location {
                timestamp: now,
                fix: fix.map(|(lat, lon)| Fix { lat, lon }),
                velocity: velocity.map(|(speed, heading)| Velocity { speed, heading }),
            });
        }
        Input::Wake => {
            race.on_timer(now);
        }
        Input::Event(e) => {
            race.on_event(now, &e);
        }
    }
}

fn f(s: &Json, k: &str) -> f64 {
    s[k].as_f64().unwrap_or_else(|| panic!("{k} in {s}"))
}

fn i(s: &Json, k: &str) -> i64 {
    s[k].as_i64().unwrap_or_else(|| panic!("{k} in {s}"))
}

fn rad(d: f64) -> f64 {
    d * PI / 180.0
}

/// A line end of the reference: where it was set (its `location` then,
/// radians), and whether that was before any fix (the (0, 0) sentinel).
#[derive(Clone, Copy, Debug)]
struct End {
    lat: f64,
    lon: f64,
    sentinel: bool,
}

/// Both engines, fed the same history, compared after every evaluation.
struct Pair {
    vm: Vm,
    old: Race,
    /// Every evaluation delivered: (time, input), wakes included.
    delivered: Vec<(u64, Input)>,
    /// The reference's line ends, as it set them.
    stbd: Option<End>,
    port: Option<End>,
    /// Whether the reference has predicted a crossing since both ends were
    /// last real (set after a fix): until then its crossing is a sentinel.
    predicted: bool,
    /// GPS updates that predicted a crossing (the line computation).
    line_updates: usize,
    /// The last velocity delivered, and whether the reference has zeroed
    /// its heading since (the gun).
    velocity: Option<(f64, f64)>,
    heading_zeroed: bool,
    /// Evaluations while racing where the reference's heading is its 0 from
    /// the gun and the view's is the last delivered.
    heading_kept: usize,
    /// Evaluations where the crossing time differs within the tolerance.
    within_tolerance: Vec<String>,
}

impl Pair {
    fn new() -> Self {
        Pair {
            vm: Vm::new(),
            old: Race::default(),
            delivered: Vec::new(),
            stbd: None,
            port: None,
            predicted: false,
            line_updates: 0,
            velocity: None,
            heading_zeroed: false,
            heading_kept: 0,
            within_tolerance: Vec::new(),
        }
    }

    /// The host: the timer, if due by `now`, at its time; then `input`; then
    /// the timer again, if that put it in the past.
    fn input(&mut self, now: u64, input: Input) {
        self.wakes_due(now);
        self.deliver(now, input);
        self.wakes_due(now);
    }

    fn wakes_due(&mut self, now: u64) {
        while let Some(at) = self.vm.wake
            && at <= now
        {
            // at its time, or at once if that's already past
            let last = self.delivered.last().map_or(0, |d| d.0);
            self.deliver(at.max(last), Input::Wake);
        }
    }

    fn both_real(&self) -> bool {
        matches!((self.stbd, self.port), (Some(s), Some(p)) if !s.sentinel && !p.sentinel)
    }

    fn deliver(&mut self, now: u64, input: Input) {
        let n = self.delivered.len();
        self.delivered.push((now, input));
        let before = self.vm.state.clone();
        let racing_before = matches!(self.old.view().phase, Phase::Racing { .. });
        let counting_before = matches!(self.old.view().phase, Phase::InSequence { .. });
        let both_real_before = self.both_real();
        let had_fix = !self.vm.gps.is_empty();

        let s = self.vm.eval(now, &input);
        old_eval(&mut self.old, now, &input);
        let where_ = || format!("evaluation {n} at T0{:+} ({input:?})\nnew state: {s}\nprevious: {before}", now as i64 - T0 as i64);

        // What the reference did, from its own fields.
        let here = End { lat: self.old.location.lat, lon: self.old.location.lon, sentinel: !had_fix };
        match input {
            Input::Gps { fix, velocity } => {
                if let Some(v) = velocity {
                    self.velocity = Some(v);
                    self.heading_zeroed = false;
                }
                if fix.is_some() && velocity.is_some() && !racing_before && self.both_real() {
                    self.predicted = true;
                    self.line_updates += 1;
                }
            }
            Input::Wake => self.heading_zeroed |= counting_before,
            Input::Event(RaceEvent::LineStbd) => self.stbd = Some(here),
            Input::Event(RaceEvent::LinePort) => self.port = Some(here),
            Input::Event(_) => {}
        }
        if self.both_real() && !both_real_before {
            self.predicted = false; // the reference's crossing is (0, 0) again
        }

        // The timer.
        assert_eq!(self.vm.wake, self.old.next_timer(), "the timers differ: {}", where_());

        // The view: the projection of the state that clients receive, and
        // nothing else.
        let v = self.vm.view.clone();
        let ov = v.as_object().unwrap();
        assert!(
            ov.keys().all(|k| ["phase", "line", "crossing", "speed", "heading"].contains(&k.as_str())),
            "the view's fields: {v}\n{}",
            where_()
        );
        let racing = s["phase"].get("racing").is_some();

        // The phase and its gun: the view's is the state's, and the
        // reference's.
        let view = self.old.view();
        let phase = &s["phase"];
        assert_eq!(phase.as_object().unwrap().len(), 1, "one phase: {}", where_());
        assert_eq!(&v["phase"], phase, "the view's phase is the state's: {v}\n{}", where_());
        let (line_view, ref_heading) = match view.phase {
            Phase::Active { line } => {
                assert_eq!(phase["idle"], json!({}), "idle: {}", where_());
                (Some(line), None)
            }
            Phase::InSequence { start_time, line } => {
                assert_eq!(i(&phase["countdown"], "gun"), start_time as i64, "countdown: {}", where_());
                (Some(line), None)
            }
            Phase::Racing { start_time, heading } => {
                assert_eq!(i(&phase["racing"], "gun"), start_time as i64, "racing: {}", where_());
                (None, Some(heading)) // the reference doesn't show the line while racing
            }
        };

        // The line's ends: as the reference set them, or absent where it
        // holds the (0, 0) sentinel.
        let line = &s["line"];
        for (name, end) in [("stbd", self.stbd), ("port", self.port)] {
            match end {
                Some(e) if !e.sentinel => {
                    let got = &line[name];
                    assert!(got.is_object(), "{name} is set: {}", where_());
                    assert_eq!(rad(f(got, "lat")).to_bits(), e.lat.to_bits(), "{name}.lat: {}", where_());
                    assert_eq!(rad(f(got, "lon")).to_bits(), e.lon.to_bits(), "{name}.lon: {}", where_());
                }
                Some(e) => {
                    assert_eq!((e.lat, e.lon), (0.0, 0.0), "the reference's sentinel: {}", where_());
                    assert!(line.get(name).is_none(), "{name} set before any fix stays absent: {}", where_());
                }
                None => assert!(line.get(name).is_none(), "{name} isn't set: {}", where_()),
            }
        }
        if let Some(lv) = line_view {
            let want = match (self.stbd.is_some(), self.port.is_some()) {
                (false, false) => matches!(lv, LineView::None),
                (true, false) => matches!(lv, LineView::Stbd),
                (false, true) => matches!(lv, LineView::Port),
                (true, true) => matches!(lv, LineView::Both { .. }),
            };
            assert!(want, "the test's idea of the reference's ends ({:?}, {:?}) isn't its view {lv:?}: {}", self.stbd, self.port, where_());
        }

        // The view's line: which ends the state has, in every phase. Against
        // the reference's tag (not shown while racing): an end it set before
        // any fix is set in its tag and unset here (the (0, 0) sentinel);
        // every other end is set in both or neither.
        let vline = &v["line"];
        assert_eq!(vline.as_object().unwrap().len(), 2, "the view's line is {{stbd, port}}: {v}\n{}", where_());
        let ours = |name: &str| vline[name].as_bool().unwrap_or_else(|| panic!("view line.{name} is a bool: {v}\n{}", where_()));
        let (v_stbd, v_port) = (ours("stbd"), ours("port"));
        assert_eq!((v_stbd, v_port), (line.get("stbd").is_some(), line.get("port").is_some()), "the view's line is the state's ends: {}", where_());
        if let Some(lv) = line_view {
            let (r_stbd, r_port) = match lv {
                LineView::None => (false, false),
                LineView::Stbd => (true, false),
                LineView::Port => (false, true),
                LineView::Both { .. } => (true, true),
            };
            for (name, end, r, o) in [("stbd", self.stbd, r_stbd, v_stbd), ("port", self.port, r_port, v_port)] {
                if end.is_some_and(|e| e.sentinel) {
                    assert!(r && !o, "{name} set before any fix: the reference's tag has it ({lv:?}), the view doesn't: {v}\n{}", where_());
                } else {
                    assert_eq!(o, r, "view line.{name} against the reference's {lv:?}: {v}\n{}", where_());
                }
            }
        }

        // The crossing time: where the reference has predicted one, the same
        // (within the tolerance); elsewhere absent. In the state while
        // racing too; in the view, not while racing.
        let crossing = line.get("crossing");
        if !(self.predicted && self.both_real()) {
            assert!(crossing.is_none(), "no crossing predicted: {}", where_());
        } else {
            let c = crossing.unwrap_or_else(|| panic!("a crossing is predicted: {}", where_()));
            let c = c.as_i64().unwrap_or_else(|| panic!("the crossing is a time: {}", where_()));
            match line_view {
                Some(LineView::Both { time, .. }) => {
                    let d = c - time as i64;
                    assert!(d.abs() <= TIME_TOLERANCE_MS, "time: {d} ms off: {}", where_());
                    if d != 0 {
                        self.within_tolerance.push(format!("evaluation {n}: extreme-race {time}, VM {c}"));
                    }
                }
                Some(lv) => panic!("predicted, but the reference's line is {lv:?}: {}", where_()),
                // Racing: neither engine updates it; it is the one compared before.
                None => assert_eq!(Some(c), before["line"]["crossing"].as_i64(), "the crossing is kept while racing: {}", where_()),
            }
        }
        if racing {
            assert!(v.get("crossing").is_none(), "no crossing in the view while racing: {v}\n{}", where_());
        } else {
            assert_eq!(v.get("crossing"), crossing, "the view's crossing is the state's: {v}\n{}", where_());
        }

        // The velocity: the last delivered, absent before the first.
        let (rs, rh) = (self.old.velocity.speed, self.old.velocity.heading);
        match self.velocity {
            None => {
                assert!(s.get("velocity").is_none(), "no velocity yet: {}", where_());
                assert_eq!((rs, rh), (0.0, 0.0), "the reference's sentinel: {}", where_());
            }
            Some((speed, heading)) => {
                let sv = &s["velocity"];
                assert_eq!((f(sv, "speed").to_bits(), f(sv, "heading").to_bits()), (speed.to_bits(), heading.to_bits()), "velocity: {}", where_());
                assert_eq!(rs.to_bits(), speed.to_bits(), "the reference's speed: {}", where_());
                let rh_want = if self.heading_zeroed { 0.0 } else { heading };
                assert_eq!(rh.to_bits(), rh_want.to_bits(), "the reference's heading: {}", where_());
            }
        }

        // The view's speed, in every phase: the last delivered, against the
        // reference's (an f32); absent before the first, where the
        // reference's is 0.
        match self.velocity {
            None => {
                assert!(v.get("speed").is_none(), "no speed yet: {v}\n{}", where_());
                assert_eq!(view.speed.to_bits(), 0f32.to_bits(), "the reference's speed sentinel: {}", where_());
            }
            Some((speed, _)) => {
                let vs = f(&v, "speed");
                assert_eq!(vs.to_bits(), speed.to_bits(), "the view's speed: {v}\n{}", where_());
                assert_eq!((vs as f32).to_bits(), view.speed.to_bits(), "the view's speed against the reference's: {v}\n{}", where_());
            }
        }

        // The view's heading: while racing only, the last delivered. The
        // reference's: 0 from the gun until the next velocity (there, ours
        // is the last heading before the gun), and 0 before any velocity
        // (there, ours is absent); otherwise the same, as an f32.
        match (ref_heading, self.velocity) {
            (None, _) => assert!(v.get("heading").is_none(), "no heading in the view unless racing: {v}\n{}", where_()),
            (Some(rh), None) => {
                assert!(v.get("heading").is_none(), "racing, no velocity yet: no heading: {v}\n{}", where_());
                assert_eq!(rh.to_bits(), 0f32.to_bits(), "the reference's heading sentinel: {}", where_());
            }
            (Some(rh), Some((_, heading))) => {
                let vh = f(&v, "heading");
                assert_eq!(vh.to_bits(), heading.to_bits(), "the view's heading is the last delivered: {v}\n{}", where_());
                if self.heading_zeroed {
                    assert_eq!(rh.to_bits(), 0f32.to_bits(), "the reference zeroes the heading at the gun: {}", where_());
                    self.heading_kept += 1;
                } else {
                    assert_eq!((vh as f32).to_bits(), rh.to_bits(), "the view's heading against the reference's: {v}\n{}", where_());
                }
            }
        }
        assert_eq!(racing, ref_heading.is_some(), "racing in both or neither: {}", where_());
    }

    fn report(&self, what: &str) {
        let wakes = self.delivered.iter().filter(|d| matches!(d.1, Input::Wake)).count();
        eprintln!(
            "{what}: {} evaluations ({wakes} wakes), {} GPS updates predicting a crossing; racing with the reference's heading zeroed at the gun (ours the last): {}; crossing times off by up to {TIME_TOLERANCE_MS} ms: {}{}",
            self.delivered.len(),
            self.line_updates,
            self.heading_kept,
            self.within_tolerance.len(),
            self.within_tolerance.iter().map(|w| format!("\n  {w}")).collect::<String>()
        );
    }
}

/// The line's two ends, ~185 m apart east-west off Sydney Heads; the
/// starboard end is the eastern one (to the right heading north).
const STBD: (f64, f64) = (-33.85, 151.252);
const PORT: (f64, f64) = (-33.85, 151.250);

fn fix(lat: f64, lon: f64, speed: f64, heading: f64) -> Input {
    Input::Gps { fix: Some((lat, lon)), velocity: Some((speed, heading)) }
}

fn bump(seconds: i32, timestamp: u64) -> Input {
    Input::Event(RaceEvent::BumpSeq { seconds, timestamp })
}

fn ev(e: RaceEvent) -> Input {
    Input::Event(e)
}

/// A race: set the line, approach it on various courses, run a sequence with
/// every kind of bump, the gun, racing, finish; then the odd cases.
#[test]
fn the_scripted_race() {
    let mut p = Pair::new();
    let mut t = T0;
    let mut at = |p: &mut Pair, dt: u64, input: Input| {
        t += dt;
        p.input(t, input);
        t
    };
    at(&mut p, 0, ev(RaceEvent::RaceFinish)); // the first event (state nil): finish while idle, nothing
    at(&mut p, 10, ev(RaceEvent::LinePort)); // before any fix: the reference's (0, 0), absent here
    at(&mut p, 500, fix(STBD.0, STBD.1, 4.0, 90.0));
    at(&mut p, 500, ev(RaceEvent::LineStbd)); // the reference: Port -> Both against (0, 0)
    at(&mut p, 500, fix(PORT.0, PORT.1, 4.0, 270.0)); // ... and predicts against it; not here
    at(&mut p, 500, ev(RaceEvent::LinePort)); // both ends real now
    // Off to the south of the line, then back towards it on every kind of
    // course: at it (between the ends), at either end, beside, away, abeam;
    // at speed, stopped, and barely moving.
    for k in 0..40u64 {
        let lat = -33.8530 + k as f64 * 0.00007;
        let lon = 151.2490 + (k % 7) as f64 * 0.0005;
        let heading = [0.0, 10.0, 350.0, 30.0, 330.0, 90.0, 180.0, 270.0, 45.0, 315.0, 5.5, 359.9][k as usize % 12];
        let speed = [5.0, 6.2, 0.0, 3.3, 0.00001, 12.0][k as usize % 6];
        at(&mut p, 1000, fix(lat, lon, speed, heading));
    }
    at(&mut p, 1000, Input::Gps { fix: Some((-33.852, 151.251)), velocity: None }); // moves, no line update
    at(&mut p, 1000, Input::Gps { fix: None, velocity: Some((4.4, 3.0)) }); // velocity only: no line update
    at(&mut p, 1000, Input::Gps { fix: None, velocity: None }); // nothing at all
    // A sequence: 5 minutes from a tap 200 ms before the event arrives.
    let t1 = at(&mut p, 1000, fix(-33.8516, 151.2508, 5.0, 0.0));
    at(&mut p, 500, bump(300, t1 + 300));
    for k in 0..6u64 {
        at(&mut p, 1000, fix(-33.8515 + k as f64 * 0.0001, 151.2508, 5.0, 2.0 * k as f64));
    }
    let t2 = at(&mut p, 333, bump(0, t1 + 6500)); // sync: the time left (from the tap) down to a whole minute
    at(&mut p, 500, bump(-60, t2)); // a minute later
    at(&mut p, 500, bump(30, t2 + 500)); // 30 s earlier
    at(&mut p, 500, ev(RaceEvent::LineStbd)); // move the stbd end: the crossing is kept ...
    let t3 = at(&mut p, 500, fix(-33.8512, 151.2511, 5.5, 1.0)); // ... until this update
    at(&mut p, 500, bump(0, t3 + 300)); // sync again
    // GPS at irregular intervals (0.2 s to 17 s, gaps of 10 s and more),
    // past the gun: the host's timer fires it at the gun exactly, between
    // two fixes.
    let gaps = [1000, 317, 1200, 10_000, 200, 4100, 17_000, 900, 1000, 12_345];
    let mut k = 0;
    while p.old.next_timer().is_some() || k < 5 {
        at(&mut p, gaps[k % gaps.len()], fix(-33.8505 + k as f64 * 0.000001, 151.2509, 4.5, 0.5));
        k += 1;
    }
    at(&mut p, 2000, fix(-33.8504, 151.2509, 4.6, 1.5)); // the reference's heading comes back
    let gun = p.delivered.iter().find(|d| matches!(d.1, Input::Wake)).expect("the gun fired").0;
    let Phase::Racing { start_time, .. } = p.old.view().phase else { panic!("not racing") };
    assert_eq!(gun, start_time, "the gun fired at its time exactly");
    at(&mut p, 400, ev(RaceEvent::LinePort)); // the line can still be moved while racing
    at(&mut p, 500, ev(RaceEvent::RaceFinish));
    at(&mut p, 500, ev(RaceEvent::RaceFinish)); // again: nothing
    at(&mut p, 500, fix(-33.8510, 151.2510, 5.0, 0.0)); // idle again: line updates resume
    at(&mut p, 500, bump(-10, t3)); // a start already past: the gun at once
    let t4 = at(&mut p, 500, bump(120, t3 + 2000)); // racing: a new sequence
    at(&mut p, 500, ev(RaceEvent::RaceFinish));
    at(&mut p, 500, bump(60, t4 + 1000)); // idle: a sequence, a minute off
    assert!(p.vm.state["phase"]["countdown"].is_object(), "counting down: {}", p.vm.state);
    at(&mut p, 500, bump(i32::MAX, t4)); // counting down, far too early: clamps to 0, the gun at once
    assert_eq!(p.vm.state["phase"], json!({"racing": {"gun": 0}}), "clamped to the epoch, and fired");
    at(&mut p, 500, bump(i32::MAX, t3)); // racing: a new sequence, as far off as can be
    at(&mut p, 500, bump(i32::MIN, t3)); // counting down: the largest delay
    at(&mut p, 500, ev(RaceEvent::RaceFinish)); // abort: the timer goes
    at(&mut p, 500, bump(0, t3)); // 0 from idle: the gun now
    at(&mut p, 500, ev(RaceEvent::RaceFinish));
    at(&mut p, 500, bump(0, 0)); // a tap at the epoch
    at(&mut p, 500, ev(RaceEvent::RaceFinish));
    at(&mut p, 500, bump(-5, 1000)); // before the epoch: clamps to 0
    at(&mut p, 500, bump(2, t3)); // racing again: a new sequence, 2 s from an old tap: the gun at once
    p.report("scripted race");
    assert!(p.line_updates >= 40, "the script should exercise the line computation");
    assert!(p.heading_kept >= 1, "the script should race with the reference's heading zeroed at the gun");
}

/// A deterministic random walk around the line: thousands of GPS updates
/// from every direction and speed, with the line set, before and during a
/// sequence and after its gun; both engines compared after each.
#[test]
fn a_random_walk() {
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x % 1_000_000) as f64 / 1_000_000.0
    };
    let mut p = Pair::new();
    p.input(T0, fix(STBD.0, STBD.1, 3.0, 0.0));
    p.input(T0 + 100, ev(RaceEvent::LineStbd));
    p.input(T0 + 600, fix(PORT.0, PORT.1, 3.0, 0.0));
    p.input(T0 + 700, ev(RaceEvent::LinePort));
    let mut t = T0 + 1000;
    for k in 0..3000u64 {
        // within ~1 km of the line, any heading, 0-15 kn
        let (lat, lon) = (-33.85 + (rnd() - 0.6) * 0.02, 151.251 + (rnd() - 0.5) * 0.02);
        p.input(t, fix(lat, lon, rnd() * 15.0, rnd() * 360.0));
        // irregular: mostly 0.1-2 s, one in twenty 10-30 s
        t += if rnd() < 0.05 { 10_000 + (rnd() * 20_000.0) as u64 } else { 100 + (rnd() * 1900.0) as u64 };
        if k % 500 == 250 {
            p.input(t - 50, bump(60 + k as i32 / 10, t - 60)); // a sequence; its gun among the fixes
        }
    }
    p.report("random walk");
    assert!(p.delivered.iter().any(|d| matches!(d.1, Input::Wake)), "the gun should fire");
}

/// The gun is the wake: a fix half a second after the start doesn't fire
/// it, the timer has already, at the start time.
#[test]
fn the_gun_is_the_wake() {
    let mut p = Pair::new();
    p.input(T0, bump(1, T0));
    p.input(T0 + 1500, fix(-33.85, 151.25, 5.0, 45.0));
    assert_eq!(p.delivered[1].0, T0 + 1000);
    assert!(matches!(p.delivered[1].1, Input::Wake));
    assert!(matches!(p.old.view().phase, Phase::Racing { start_time, .. } if start_time == T0 + 1000));
    assert_eq!(p.vm.state["phase"], json!({"racing": {"gun": T0 + 1000}}));
}

/// A stale wake (the host's timer raced with a race_finish) is ignored, as
/// the old on_timer ignores one outside a sequence.
#[test]
fn a_stale_wake() {
    let mut p = Pair::new();
    p.input(T0, bump(60, T0));
    p.input(T0 + 1000, ev(RaceEvent::RaceFinish));
    let before = p.vm.state.clone();
    p.deliver(T0 + 60_000, Input::Wake);
    assert_eq!(p.vm.state, before, "a stale wake changes nothing");
    assert_eq!(p.vm.state["phase"], json!({"idle": {}}));
}

/// A wake arriving in idle with everything set (the line, a crossing, a
/// velocity) is a no-op: the state is exactly what it was.
#[test]
fn a_wake_in_idle_changes_nothing() {
    let mut p = Pair::new();
    p.input(T0, fix(STBD.0, STBD.1, 3.0, 0.0));
    p.input(T0 + 100, ev(RaceEvent::LineStbd));
    p.input(T0 + 600, fix(PORT.0, PORT.1, 3.0, 0.0));
    p.input(T0 + 700, ev(RaceEvent::LinePort));
    p.input(T0 + 1700, fix(-33.853, 151.251, 5.0, 0.0));
    let before = p.vm.state.clone();
    let view_before = p.vm.view.clone();
    assert!(before["line"].get("crossing").is_some() && before.get("velocity").is_some());
    assert!(view_before.get("crossing").is_some() && view_before.get("speed").is_some(), "{view_before}");
    p.deliver(T0 + 2000, Input::Wake);
    assert_eq!(p.vm.state, before, "a wake in idle changes nothing");
    assert_eq!(p.vm.view, view_before, "nor the view: the crossing and the speed come from the state");
    assert_eq!(p.vm.wake, None);
}

/// Each kind of event as the very first (state nil), then the line and a
/// sequence after it, compared throughout.
#[test]
fn each_event_first() {
    let firsts = [
        ("gps with a fix and a velocity", fix(-33.853, 151.251, 5.0, 0.0)),
        ("gps with a fix only", Input::Gps { fix: Some((-33.853, 151.251)), velocity: None }),
        ("gps with a velocity only", Input::Gps { fix: None, velocity: Some((5.0, 10.0)) }),
        ("gps with nothing", Input::Gps { fix: None, velocity: None }),
        ("wake", Input::Wake),
        ("line_stbd", ev(RaceEvent::LineStbd)),
        ("line_port", ev(RaceEvent::LinePort)),
        ("bump_seq", bump(30, T0)),
        ("bump_seq, sync", bump(0, T0)),
        ("bump_seq, negative", bump(-30, T0)),
        ("race_finish", ev(RaceEvent::RaceFinish)),
    ];
    for (what, first) in firsts {
        let mut p = Pair::new();
        p.input(T0, first);
        let s = &p.vm.state;
        let line_empty = s["line"].as_object().unwrap().is_empty();
        assert!(line_empty, "{what}: no line yet: {s}");
        p.input(T0 + 500, ev(RaceEvent::RaceFinish)); // ends a sequence or race the first event started
        p.input(T0 + 1000, fix(STBD.0, STBD.1, 3.0, 0.0));
        p.input(T0 + 1100, ev(RaceEvent::LineStbd));
        p.input(T0 + 1600, fix(PORT.0, PORT.1, 3.0, 0.0));
        p.input(T0 + 1700, ev(RaceEvent::LinePort));
        p.input(T0 + 2700, fix(-33.853, 151.251, 5.0, 0.0));
        p.input(T0 + 3000, bump(5, T0 + 3000));
        p.input(T0 + 9000, fix(-33.852, 151.251, 5.0, 0.0));
        p.input(T0 + 9500, ev(RaceEvent::RaceFinish));
        assert!(p.line_updates >= 1, "{what}: a crossing was predicted");
        assert!(p.delivered.iter().any(|d| matches!(d.1, Input::Wake)), "{what}: the gun fired");
    }
}

/// samples.json: PROFILE.md §5's smoke runs (a GPS update with nothing as
/// the first event, then a wake), then every kind of event, as `langc check
/// --samples` would run them if it had the host's builtins and chained
/// `state` from the output's `state`: the profile runs them here. Logs the
/// instructions of each evaluation.
#[test]
fn samples() {
    let samples: Json = serde_json::from_str(include_str!("../samples.json")).unwrap();
    let mut vm = Vm::new();
    for (n, r) in samples["runs"].as_array().unwrap().iter().enumerate() {
        let mut inputs = r["inputs"].as_object().unwrap().clone();
        match r.get("chain") {
            Some(c) => {
                assert_eq!(c, &json!({"state": "output.state"}), "run {n}: `state` is chained from the output's `state`");
                inputs.insert("state".into(), vm.state.clone());
            }
            None => assert_eq!(inputs["state"], Json::Null, "run {n}: an unchained run starts from nil"),
        }
        match run(&vm.program, &vm.iface, &inputs, &sailing::BUILTINS).unwrap() {
            RunResult::Ok { value, stack_high, steps: instructions } => {
                vm.accept(&plain(&value));
                let kind = inputs["event"].as_object().unwrap().keys().next().unwrap().clone();
                eprintln!("run {n} ({kind}): {instructions} instructions, stack {stack_high}: {}\n  view {} wake {:?}", vm.state, vm.view, vm.wake);
            }
            RunResult::Failed { code, span } => panic!("run {n}: {code} at {span:?}"),
        }
    }
    let s = &vm.state;
    assert_eq!(s["phase"], json!({"idle": {}}));
    let line = &s["line"];
    assert!(line.get("stbd").is_some() && line.get("port").is_some() && line.get("crossing").is_some(), "{s}");
    let v = &vm.view;
    assert_eq!(v["phase"], json!({"idle": {}}));
    assert_eq!(v["line"], json!({"stbd": true, "port": true}));
    assert_eq!(v.get("crossing"), line.get("crossing"), "{v}");
    assert!(v.get("speed").is_some() && v.get("heading").is_none(), "{v}");
    assert_eq!(vm.wake, None);
}

#[test]
fn interface_json_is_up_to_date() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/interface.json");
    let want = serde_json::to_string_pretty(&engine_race::interface()).unwrap() + "\n";
    if std::env::var_os("UPDATE").is_some() {
        std::fs::write(path, &want).unwrap();
    }
    let got = std::fs::read_to_string(path).unwrap_or_default();
    assert!(got == want, "interface.json is stale: run `UPDATE=1 cargo test`");
}

/// The end of engine.expr's `next` binding: the output, `{state, view,
/// wake?}`, follows it.
const NEXT_END: &str = "\n);\n";

/// The smallest view the view schema accepts (the optional fields absent).
const MIN_VIEW: &str = "{phase: next.phase, line: {stbd: false, port: false}}";

/// engine.expr with its output replaced by `output`.
fn with_output(output: &str) -> String {
    let p = engine_race::PROGRAM;
    let at = p.rfind(NEXT_END).unwrap_or_else(|| panic!("engine.expr's `next` binding ends with {NEXT_END:?}")) + NEXT_END.len();
    assert!(p[at..].starts_with("{state: next,"), "engine.expr's output follows `next`: {:?}", &p[at..]);
    format!("{}{output}\n", &p[..at])
}

fn compile_errors(source: &str, iface: &Json) -> Vec<compiler::diag::Diag> {
    match compile(source, iface, &Default::default()) {
        Ok(_) => panic!("compiled, but shouldn't have:\n{source}"),
        Err(d) => d,
    }
}

/// The output is the contract: a field it doesn't declare is a compile
/// error, the checker's `E-FIELD`. Without a view schema, `view` is one.
#[test]
fn an_undeclared_output_field_is_rejected() {
    let iface = engine_race::interface();
    let output = format!("{{state: next, view: {MIN_VIEW}, wake: next.phase.countdown?.gun, extra: 1}}");
    let d = compile_errors(&with_output(&output), &iface);
    assert!(d.iter().any(|d| d.rule == "E-FIELD" && d.message == "the output has no field `extra`"), "{output}: {d:#?}");

    let state: Json = serde_json::from_str(engine_race::STATE_SCHEMA).unwrap();
    let event: Json = serde_json::from_str(engine_race::EVENT_SCHEMA).unwrap();
    let no_view = engine_race::engine_interface(&state, None, &event);
    let output = "{state: next, view: next, wake: nil}";
    let d = compile_errors(&with_output(output), &no_view);
    assert!(d.iter().any(|d| d.rule == "E-FIELD" && d.message == "the output has no field `view`"), "{output}: {d:#?}");

    let d = compile_errors(&with_output(&format!("{{view: {MIN_VIEW}, wake: next.phase.countdown?.gun}}")), &iface);
    assert!(d.iter().any(|d| d.rule == "E-FIELD" && d.message == "the output is missing the field `state`"), "{d:#?}");
}

/// With a view schema, `view` is in the output and required: Race's own,
/// and any other.
#[test]
fn a_view_schema_makes_view_required() {
    let iface = engine_race::interface();
    assert_eq!(iface["output"]["required"], json!(["state", "view"]));
    let d = compile_errors(&with_output("{state: next, wake: next.phase.countdown?.gun}"), &iface);
    assert!(d.iter().any(|d| d.rule == "E-FIELD" && d.message == "the output is missing the field `view`"), "{d:#?}");
    compile(engine_race::PROGRAM, &iface, &Default::default()).unwrap_or_else(|d| panic!("{d:#?}"));

    let state: Json = serde_json::from_str(engine_race::STATE_SCHEMA).unwrap();
    let event: Json = serde_json::from_str(engine_race::EVENT_SCHEMA).unwrap();
    let view = json!({"type": "object", "properties": {"gun": {"type": "integer", "format": "int64"}}});
    let iface = engine_race::engine_interface(&state, Some(&view), &event);
    assert_eq!(iface["output"]["required"], json!(["state", "view"]));
    let d = compile_errors(&with_output("{state: next, wake: next.phase.countdown?.gun}"), &iface);
    assert!(d.iter().any(|d| d.rule == "E-FIELD" && d.message == "the output is missing the field `view`"), "{d:#?}");
    let with_view = with_output("{state: next, view: {gun: next.phase.countdown?.gun}, wake: next.phase.countdown?.gun}");
    compile(&with_view, &iface, &Default::default()).unwrap_or_else(|d| panic!("{d:#?}"));
}

/// A nil `wake` is an absent one: no timer. Explicitly (`wake: nil`, here
/// while counting down, so only the output decides), and Race's own
/// `next.phase.countdown?.gun` while idle.
#[test]
fn a_nil_wake_is_no_timer() {
    let iface = engine_race::interface();
    let first = |source: &str, event: Json| {
        let program = compile(source, &iface, &Default::default()).unwrap_or_else(|d| panic!("{d:#?}")).bytes;
        let mut inputs = Map::new();
        inputs.insert("state".into(), Json::Null);
        inputs.insert("event".into(), event);
        inputs.insert("timestamp".into(), json!(T0));
        inputs.insert("gps".into(), json!([]));
        match run(&program, &iface, &inputs, &sailing::BUILTINS).unwrap() {
            RunResult::Ok { value, .. } => value,
            RunResult::Failed { code, span } => panic!("{code} at {span:?}"),
        }
    };
    let out = first(&with_output(&format!("{{state: next, view: {MIN_VIEW}, wake: nil}}")), json!({"bump_seq": {"seconds": 60, "tapped_at": T0}}));
    assert!(out["fields"].get("wake").is_none(), "no wake: {out}");
    assert_eq!(plain(&out)["state"]["phase"], json!({"countdown": {"gun": T0 + 60_000}}));
    let out = first(engine_race::PROGRAM, json!({"race_finish": {}}));
    assert!(out["fields"].get("wake").is_none(), "idle, no wake: {out}");
    assert_eq!(wake_of(&plain(&out)), None);
    let out = first(engine_race::PROGRAM, json!({"bump_seq": {"seconds": 60, "tapped_at": T0}}));
    assert_eq!(wake_of(&plain(&out)), Some(T0 + 60_000), "counting down, the gun: {out}");
}

/// The tooling refuses an author's event named like a built-in one.
#[test]
#[should_panic(expected = "`wake` is a built-in event")]
fn an_author_event_cant_be_named_wake() {
    engine_race::event_schema(&json!({"oneOf": [{"properties": {"wake": {"type": "object"}}, "required": ["wake"]}]}));
}
