use serde::{Deserialize, Serialize, Serializer};
use versioned_derive::Delta;
use versioned::Atomic;

use crate::engine_traits::EventHandler;
use crate::closure;
use ::serde::ser::SerializeStruct;

#[derive(Delta, Copy, Clone, PartialEq, Default)]
pub struct Race {
    location: Location,
    line: Line,
    state: State,
}

#[derive(Delta, Default, Copy, Clone, PartialEq)]
enum State {
    #[default]
    Setup,
    Idle,
    InSequence{ start: f64 },
    Racing{ start: f64 },
}


#[derive(Serialize, Copy, Clone, PartialEq, Default)]
pub struct Location {
    lat: f64,
    lon: f64,
}
impl Atomic for Location {}

#[derive(Delta, Copy, Clone, PartialEq, Default)]
pub enum Line {
    #[default]
    None,

    // #[delta(skip_fields)]
    Stbd{location: Location},

    // #[delta(skip_fields)]
    Port{location: Location},

    Both{
        time: f64, 
        point: u8,

        // #[delta(skip)]
        stbd: Location, 

        // #[delta(skip)]
        port: Location, 
    },
}


#[derive(Deserialize)]
pub enum EventType {
    SetupPushOff,

    LineStbd,
    LinePort,

    IdleSeq { seconds: f64 },
    SeqBump { seconds: f64 },

    RaceFinish,
}

#[derive(Deserialize)]
pub struct Event {
    pub timestamp: f64,
    pub event: EventType,
}

closure! {Race,
    pub RaceCallbacks {
        Start(()),
    }
}

impl EventHandler for Race {
    type Event = Event;
    type Callbacks = RaceCallbacks;

    fn handle_event(&mut self, event: Self::Event, sleep: &dyn FnMut(u32, RaceCallbacks)) -> Result<(), &'static str> {
        match event.event {
            EventType::SetupPushOff => {
                self.state = State::Idle;
                Ok(())
            },
            EventType::LineStbd => {
                self.line = Line::Stbd{location: Location::default()};
                Ok(())
            },
            EventType::LinePort => {
                self.line = Line::Port{location: Location::default()};
                Ok(())
            },
            EventType::IdleSeq { seconds } => {
                self.state = State::InSequence{ start: seconds };
                Ok(())
            },
            EventType::SeqBump { seconds } => {
                self.state = State::Racing{ start: seconds };
                Ok(())
            },
            EventType::RaceFinish => {
                self.state = State::Idle;
                Ok(())
            },
        }
    }
}



// #[cfg(test)]
// // #[cfg(feature = "std")]
// mod tests {
//     use super::*;
//     use serde_json;

//     #[test]
//     fn test_default() {
//         let event = Event{timestamp: 32.4, event: EventType::SetupPushOff};
//         match serde_json::to_string(&event) {
//             Ok(json_str) => {
//                 // Print the serialized event
//                 println!("{}", json_str);
//             },
//             Err(e) => {
//                 panic!("Failed to serialize event: {}", e);
//             },
//         }

//         // let ctx = EngineContext::default();
    
//         // let (notify_sender, notify_receiver) = unbounded::<String>();
    
//         // let notify_fn = move |event: String| {
//         //     notify_sender.send(event).unwrap();
//         // };
    
//         // ctx.set_engine(Versioned::new(Race::default(), 0), Box::new(notify_fn));
    
//         // assert_eq!(ctx.handle_event("{\"timestamp\": 0.0, \"event\": \"setup_push_off\"}"), Ok("Event scheduled"));
//         // std::thread::sleep(std::time::Duration::from_millis(100)); 
//         // assert_eq!(notify_receiver.recv().unwrap(), "\"hello\"");

//     }
// }