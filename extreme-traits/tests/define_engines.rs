use extreme_traits::{
    COMPACT_SELECT, Engine, Fix, Outcome, RawEngine, StaticFiles, Timer, Velocity, define_engines,
};
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize)]
struct Counter {
    count: u32,
}

#[derive(Deserialize)]
struct CounterEvent {
    add: u32,
}

impl Engine for Counter {
    const NAME: &'static str = "Counter";
    const STATIC_FILES: StaticFiles = &[("index.html", b"<p>counter</p>")];
    type Event<'a> = CounterEvent;

    fn location_event(&mut self, _: u64, _: Option<Fix>, _: Option<Velocity>) -> Outcome {
        Outcome::NONE
    }

    fn external_event(&mut self, timestamp: u64, event: &CounterEvent) -> Outcome {
        self.count += event.add;
        Outcome::CHANGED.with_timer(timestamp + 1000)
    }

    fn timer_event(&mut self, _: u64) -> Outcome {
        Outcome::CHANGED
    }

    fn compact_state(&self, now: u64, out: &mut [u8]) -> usize {
        out[..4].copy_from_slice(&self.count.to_le_bytes());
        out[4] = now as u8;
        5
    }

    fn compact_event(&mut self, timestamp: u64, event: &[u8]) -> Result<Outcome, ()> {
        match *event {
            [0x10, add] => Ok(Engine::external_event(
                self,
                timestamp,
                &CounterEvent { add: add as u32 },
            )),
            _ => Err(()),
        }
    }
}

#[derive(Default, Serialize)]
struct Echo<'s> {
    last: &'s str,
}

impl Engine for Echo<'static> {
    const NAME: &'static str = "Echo";
    const STATIC_FILES: StaticFiles = &[];
    type Event<'a> = &'a str;

    fn location_event(&mut self, _: u64, _: Option<Fix>, _: Option<Velocity>) -> Outcome {
        Outcome::NONE
    }

    fn external_event(&mut self, _: u64, event: &&str) -> Outcome {
        self.last = if *event == "hello" { "hello" } else { "?" };
        Outcome::CHANGED
    }

    fn timer_event(&mut self, _: u64) -> Outcome {
        Outcome::NONE
    }
}

define_engines! {
    Engines {
        Counter(Counter),
        Echo(Echo<'static>),
    }
}

fn state(e: &Engines) -> serde_json::Value {
    serde_json::from_slice(&e.serialize_state().unwrap()).unwrap()
}

#[test]
fn starts_in_selector_and_lists_engines() {
    let e = Engines::default();
    assert_eq!(e.kind(), "Selector");
    assert_eq!(
        state(&e),
        serde_json::json!({ "engines": ["Counter", "Echo"] })
    );
    assert!(e.get_static("index.html").is_some());
}

#[test]
fn select_switches_engine_and_cancels_timer() {
    let mut e = Engines::default();
    let outcome = e.external_event(0, br#"{"Select":"Counter"}"#).unwrap();
    assert_eq!(
        outcome,
        Outcome {
            changed: true,
            timer: Timer::Cancel
        }
    );
    assert_eq!(e.kind(), "Counter");
    assert_eq!(state(&e), serde_json::json!({ "count": 0 }));
    assert_eq!(e.get_static("index.html"), Some(&b"<p>counter</p>"[..]));

    // unknown names return to the selector
    let _ = e.external_event(0, br#"{"Select":"Nope"}"#).unwrap();
    assert_eq!(e.kind(), "Selector");
}

#[test]
fn events_reach_only_the_active_engine() {
    let mut e = Engines::default();
    // Counter is not active: valid event, but ignored
    assert_eq!(
        e.external_event(0, br#"{"Counter":{"add":5}}"#).unwrap(),
        Outcome::NONE
    );

    let _ = e.external_event(0, br#"{"Select":"Counter"}"#).unwrap();
    let outcome = e.external_event(10, br#"{"Counter":{"add":5}}"#).unwrap();
    assert_eq!(
        outcome,
        Outcome {
            changed: true,
            timer: Timer::At(1010)
        }
    );
    assert_eq!(state(&e), serde_json::json!({ "count": 5 }));

    assert_eq!(e.timer_event(1010), Outcome::CHANGED);
}

#[test]
fn borrowed_events_work() {
    let mut e = Engines::default();
    let _ = e.external_event(0, br#"{"Select":"Echo"}"#).unwrap();
    let _ = e.external_event(0, br#"{"Echo":"hello"}"#).unwrap();
    assert_eq!(state(&e), serde_json::json!({ "last": "hello" }));
}

#[test]
fn malformed_events_are_errors() {
    let mut e = Engines::default();
    assert!(e.external_event(0, b"not json").is_err());
    assert!(e.external_event(0, br#"{"Unknown":1}"#).is_err());
    assert!(
        e.external_event(0, br#"{"Select":"Counter","Counter":{"add":1}}"#)
            .is_err()
    );
    assert!(e.external_event(0, br#"{"Counter":{"wrong":1}}"#).is_err());
    assert_eq!(e.kind(), "Selector");
}

#[test]
fn compact_protocol() {
    let mut e = Engines::default();
    assert_eq!(e.kind_code(), 0);
    // the selector has no compact body
    assert_eq!(&e.compact_state(7)[..], &[0]);

    // select by code; unknown codes return to the selector
    assert_eq!(
        e.compact_event(0, &[COMPACT_SELECT, 1]).unwrap(),
        Outcome {
            changed: true,
            timer: Timer::Cancel
        }
    );
    assert_eq!(e.kind(), "Counter");
    assert_eq!(e.kind_code(), 1);

    let outcome = e.compact_event(10, &[0x10, 5]).unwrap();
    assert_eq!(
        outcome,
        Outcome {
            changed: true,
            timer: Timer::At(1010)
        }
    );
    assert_eq!(&e.compact_state(7)[..], &[1, 5, 0, 0, 0, 7]);

    // an engine with no compact form still has its kind byte
    let _ = e.compact_event(0, &[COMPACT_SELECT, 2]).unwrap();
    assert_eq!(e.kind_code(), 2);
    assert_eq!(&e.compact_state(0)[..], &[2]);
    assert_eq!(e.compact_event(0, &[0x10, 5]), Err(()));

    let _ = e.compact_event(0, &[COMPACT_SELECT, 9]).unwrap();
    assert_eq!(e.kind(), "Selector");
    // a select needs exactly two bytes
    assert_eq!(e.compact_event(0, &[COMPACT_SELECT]), Err(()));
}
