// use serde::{Serialize, Serializer};
use serde_json;
use serde_derive::{Deserialize, Serialize};

use versioned_derive::{Versioned};
use versioned::{Versioned, VersionedValue, VersionedType, Atomic, update, DeltaType};

use crate::engine_traits::{EventHandler, EventSleepFn};

#[derive(Serialize, Versioned, Default)]
pub struct Race {
    location: Location,

    // #[serde(flatten)]
    // line: Line,

    #[serde(flatten)]
    state: State,
}

#[derive(Default, Serialize, Versioned)]
#[serde(tag = "state", rename_all="snake_case")]
enum State {
    #[default]
    Setup,
    // #[serde(flatten)]
    Idle{ line: Line },
    InSequence{ start: f64, line: Line},
    Racing{ start: f64 },
}

// impl Default for State {
//     fn default() -> Self {
//         State::Idle{ line: Line::default() }
//     }
// }

#[derive(Serialize, Clone, Default)]
pub struct Location {
    lat: f64,
    lon: f64,
}
impl Atomic for Location {}

#[derive(Serialize, Versioned, Default)]
#[serde(tag = "line", rename_all="snake_case")]
pub enum Line {
    #[default]
    None,

    #[versioned(skip_fields)]
    Stbd{location: Location},

    #[versioned(skip_fields)]
    Port{location: Location},

    Both{
        time: f64, 
        point: u8,

        #[versioned(skip)]
        stbd: Location, 

        #[versioned(skip)]
        port: Location, 
    },
}


#[derive(Deserialize)]
#[serde(tag = "event", rename_all="snake_case")]
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
    timestamp: f64,
    #[serde(flatten)]
    event: EventType,
}


impl EventHandler for Race {
    type Event = Event;
    type Update = Race;

    fn handle_event(&mut self, event: Self::Event, sleep: &EventSleepFn<Self>) -> Result<Option<Self::Update>, &'static str> {
        Ok(Some(Race::default()))
        //     Event::Sleep { value } => {
        //         sleep(2000, Box::new(move |sm: &mut Self| {
        //             sm.data += value;
        //             let result = State{value: sm.data };
        //             Some(result)
        //         }));
        //         Ok(None)
        //     }
        //     Event::Increment => {
        //         // Handle the increment event
        //         self.data += 1;
        //         let result = State{value: self.data };
        //         Ok(Some(result))
        //     }
        // }
    }
}

impl EventHandler for VersionedValue<VersionedType<Race>> {
    type Event = Event;
    type Update = DeltaType<Race>;

    fn handle_event(&mut self, event: Self::Event, sleep: &EventSleepFn<Self>) -> Result<Option<Self::Update>, &'static str> {
        match event.event {
            EventType::SetupPushOff => {
                update!(self.state, State::Idle{ line: Line::default() });
            },
            // EventType::LineStbd => {
            //     update!(self.state, State::Idle{ line: Line::Stbd{location: Location::default()} });
            //     Ok(None)
            // },
            // EventType::LinePort => {
            //     update!(self.state, State::Idle{ line: Line::Port{location: Location::default()} });
            //     Ok(None)
            // },
            // EventType::IdleSeq { seconds } => {
            //     update!(self.state, State::InSequence{ start: seconds, line: Line::default() });
            //     Ok(None)
            // },
            // EventType::SeqBump { seconds } => {
            //     update!(self.state, State::Racing{ start: seconds });
            //     Ok(None)
            // },
            // EventType::RaceFinish => {
            //     update!(self.state, State::Idle{ line: Line::default() });
            //     Ok(None)
            // },
            _ => {
                return Ok(None)
            }
        }

        Ok(Some(Race::get(self, 0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use crate::engine_context::EngineContext;

    #[test]
    fn test_default() {
        let ctx = EngineContext::default();
    
        let (notify_sender, notify_receiver) = unbounded::<String>();
    
        let notify_fn = move |event: String| {
            notify_sender.send(event).unwrap();
        };
    
        ctx.set_engine(Versioned::new(Race::default(), 0), Box::new(notify_fn));
    
        assert_eq!(ctx.handle_event("{\"timestamp\": 0.0, \"event\": \"setup_push_off\"}"), Ok("Event scheduled"));
        std::thread::sleep(std::time::Duration::from_millis(100)); 
        assert_eq!(notify_receiver.recv().unwrap(), "\"hello\"");

    }
}