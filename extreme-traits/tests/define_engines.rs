use extreme_traits::{
    Engine, Fix, MAX_STATE_SIZE, Outcome, RawEngine, StaticFiles, Timer, Velocity, define_engines,
};
use postcard_schema::{
    Schema,
    schema::{DataModelType, DataModelVariant},
};
use serde::{Deserialize, Serialize};

#[derive(Default)]
struct Counter {
    count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Schema)]
struct CounterView {
    count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Schema)]
struct CounterEvent {
    add: u32,
}

impl Engine for Counter {
    const NAME: &'static str = "Counter";
    const STATIC_FILES: StaticFiles = &[("index.html", b"<p>counter</p>")];
    type View = CounterView;
    type Event = CounterEvent;

    fn view(&self) -> CounterView {
        CounterView { count: self.count }
    }

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
}

/// An engine with an enum view and enum events.
#[derive(Default)]
struct Echo {
    last: Option<Word>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Schema)]
enum Word {
    Hello,
    Number(i16),
}

impl Engine for Echo {
    const NAME: &'static str = "Echo";
    const STATIC_FILES: StaticFiles = &[];
    type View = Option<Word>;
    type Event = Word;

    fn view(&self) -> Option<Word> {
        self.last
    }

    fn location_event(&mut self, _: u64, _: Option<Fix>, _: Option<Velocity>) -> Outcome {
        Outcome::NONE
    }

    fn external_event(&mut self, _: u64, event: &Word) -> Outcome {
        self.last = Some(*event);
        Outcome::CHANGED
    }

    fn timer_event(&mut self, _: u64) -> Outcome {
        Outcome::NONE
    }
}

define_engines! {
    Engines {
        Counter(Counter),
        Echo(Echo),
    }
}

fn state(e: &Engines) -> EngineState {
    let bytes = e.serialize_state().unwrap();
    postcard::from_bytes(&bytes).unwrap()
}

fn event(e: &EngineEvent) -> Vec<u8> {
    postcard::to_stdvec(e).unwrap()
}

#[test]
fn starts_in_selector() {
    let e = Engines::default();
    assert_eq!(e.kind(), "Selector");
    assert_eq!(e.engine_kind(), EngineKind::Selector);
    // the selector's tag, and no data
    assert_eq!(&e.serialize_state().unwrap()[..], &[0]);
    assert_eq!(state(&e), EngineState::Selector(()));
    assert!(e.get_static("index.html").is_some());
}

#[test]
fn tags_follow_declaration_order() {
    assert_eq!(event(&EngineEvent::Select(EngineKind::Selector)), [0, 0]);
    assert_eq!(event(&EngineEvent::Select(EngineKind::Counter)), [0, 1]);
    assert_eq!(event(&EngineEvent::Select(EngineKind::Echo)), [0, 2]);
    assert_eq!(event(&EngineEvent::Counter(CounterEvent { add: 300 })), [1, 0xac, 0x02]);
    assert_eq!(event(&EngineEvent::Echo(Word::Number(-2))), [2, 1, 3]);

    let names = |ty: &DataModelType| match ty {
        DataModelType::Enum(variants) => variants.iter().map(|v| v.name).collect::<Vec<_>>(),
        _ => panic!("not an enum"),
    };
    assert_eq!(names(EngineKind::SCHEMA.ty), ["Selector", "Counter", "Echo"]);
    assert_eq!(names(EngineState::SCHEMA.ty), ["Selector", "Counter", "Echo"]);
    assert_eq!(names(EngineEvent::SCHEMA.ty), ["Select", "Counter", "Echo"]);
    let DataModelType::Enum(variants) = EngineState::SCHEMA.ty else {
        unreachable!()
    };
    assert_eq!(
        variants[1].ty,
        &DataModelVariant::NewtypeVariant(CounterView::SCHEMA)
    );
}

#[test]
fn select_switches_engine_and_cancels_timer() {
    let mut e = Engines::default();
    let outcome = e
        .external_event(0, &event(&EngineEvent::Select(EngineKind::Counter)))
        .unwrap();
    assert_eq!(
        outcome,
        Outcome {
            changed: true,
            timer: Timer::Cancel
        }
    );
    assert_eq!(e.kind(), "Counter");
    assert_eq!(state(&e), EngineState::Counter(CounterView { count: 0 }));
    assert_eq!(&e.serialize_state().unwrap()[..], &[1, 0]);
    assert_eq!(e.get_static("index.html"), Some(&b"<p>counter</p>"[..]));

    let _ = e
        .external_event(0, &event(&EngineEvent::Select(EngineKind::Selector)))
        .unwrap();
    assert_eq!(e.kind(), "Selector");
}

#[test]
fn events_reach_only_the_active_engine() {
    let mut e = Engines::default();
    let add5 = event(&EngineEvent::Counter(CounterEvent { add: 5 }));
    // Counter is not active: a valid event, refused
    assert_eq!(e.external_event(0, &add5), Err(()));

    let _ = e
        .external_event(0, &event(&EngineEvent::Select(EngineKind::Counter)))
        .unwrap();
    let outcome = e.external_event(10, &add5).unwrap();
    assert_eq!(
        outcome,
        Outcome {
            changed: true,
            timer: Timer::At(1010)
        }
    );
    assert_eq!(state(&e), EngineState::Counter(CounterView { count: 5 }));

    assert_eq!(e.timer_event(1010), Outcome::CHANGED);
}

#[test]
fn enum_views_and_events() {
    let mut e = Engines::select(EngineKind::Echo);
    assert_eq!(state(&e), EngineState::Echo(None));
    assert_eq!(&e.serialize_state().unwrap()[..], &[2, 0]);
    let _ = e
        .external_event(0, &event(&EngineEvent::Echo(Word::Number(-300))))
        .unwrap();
    assert_eq!(state(&e), EngineState::Echo(Some(Word::Number(-300))));
    // tag, Some, Number, zigzag -300 = 599
    assert_eq!(&e.serialize_state().unwrap()[..], &[2, 1, 1, 0xd7, 0x04]);
}

#[test]
fn malformed_events_are_errors() {
    let mut e = Engines::default();
    assert!(e.external_event(0, b"").is_err());
    // no such variant
    assert!(e.external_event(0, &[3]).is_err());
    // no such engine kind
    assert!(e.external_event(0, &[0, 9]).is_err());
    // truncated
    assert!(e.external_event(0, &[1, 0x80]).is_err());
    // trailing bytes
    assert!(e.external_event(0, &[0, 1, 0]).is_err());
    assert_eq!(e.kind(), "Selector");
}

#[test]
fn state_size_limit() {
    // the largest Counter state: tag + a 5-byte varint
    let e = Engines::Counter(Counter { count: u32::MAX });
    assert_eq!(e.serialize_state().unwrap().len(), 6);
    assert!(6 <= MAX_STATE_SIZE);
}
