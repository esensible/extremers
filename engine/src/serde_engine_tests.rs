#[cfg(test)]
mod tests {
    use crate::*;
    use ::serde::Deserialize;
    use ::serde::Serializer;
    
    #[test]
    fn test_serde() {
        type AnEngine = EventEngine<ACore, 1>;
        type Serde = SerdeEngine<AnEngine>;

        let mut engine = Serde::default();

        assert_eq!(engine.0.0, ACore{f1: 0, f2: 0, loc: false});

        // simple event to update state
        handle_event(
            &mut engine,
            serde_json::json!(
                {
                    "event": {
                        "Event1": {
                            "value": 23
                        }
                    }
                }
            ),
            serde_json::json!({"cnt": 1, "update": {"f1": 23}}),
            None
        );

        assert_eq!(engine.0.0, ACore{f1: 23, f2: 0, loc: false});

        //
        // event with callback
        //
        handle_event(
            &mut engine,
            serde_json::json!(
                {
                    "event": {
                        "Event2": {
                            "timestamp": 1024,
                            "value": 42
                        }
                    }
                }
            ),
            serde_json::json!({"cnt": 2, "update": {}}),
            None
        );

        assert_eq!(engine.0.0, ACore{f1: 23, f2: 0, loc: false});

        // call the callback
        let mut response = [0u8; 1024];
        let result = engine.handle_sleep(&mut response, 0);
        let len = if let Some(len) = result {
            len
        } else {
            panic!("Expected Some(len)");
        };
        let result: serde_json::Value = serde_json::from_slice(&response[..len]).unwrap();
        assert_eq!(result, serde_json::json!({"cnt": 3, "update": {"f2": 42}}));

        assert_eq!(engine.0.0, ACore{f1: 23, f2: 42, loc: false});

        //
        // update location
        //
    
        let result = engine.update_location(0, Some((42.3, -113.2)), Some((0.0, 0.0)), &mut response);
        let len = if let Some(len) = result {
            len
        } else {
            panic!("Expected Some(len)");
        };
        let result: serde_json::Value = serde_json::from_slice(&response[..len]).unwrap();
        assert_eq!(result, serde_json::json!({"cnt": 4, "update": {"loc": true}}));

        assert_eq!(engine.0.0, ACore{f1: 23, f2: 42, loc: true});

        //
        // one last event, just to ensure update_location() kicked over cnt as expected
        //
        handle_event(
            &mut engine,
            serde_json::json!(
                {
                    "event": {
                        "Event1": {
                            "value": 69
                        }
                    }
                }
            ),
            serde_json::json!({"cnt": 5, "update": {"f1": 69}}),
            None
        );

        assert_eq!(engine.0.0, ACore{f1: 69, f2: 42, loc: true});

    }

    #[derive(FlatDiffSer, Copy, Clone, PartialEq, Default, Debug)]
    pub struct ACore {
        f1: u32,
        f2: u32,
        loc: bool,
    }

    #[derive(Deserialize)]
    enum SomeEvents {
        Event1{value: u32},
        Event2{value: u32, timestamp: u64},
    }

    #[derive(Deserialize)]
    pub struct Event {
        event: SomeEvents,
    }

    callbacks! {ACore,
        pub ACoreCallbacks {
            Callback(u32),
        }
    }
    
    impl ACore {
        fn callback(&mut self, arg: &u32) {
            self.f2 = *arg;
        }
    }
    
    impl EngineCore for ACore {
        type Event = Event;
        type Callbacks = ACoreCallbacks;

        fn handle_event(
            &mut self,
            event: Self::Event,
            sleep: &mut dyn FnMut(u64, Self::Callbacks) -> Result<(), &'static str>,
        ) -> Result<bool, &'static str> {
            match event.event {
                SomeEvents::Event1{value} => {
                    self.f1 = value;
                    Ok(true)
                }
                SomeEvents::Event2{value, timestamp} => {
                    sleep(timestamp, <u32>::new(ACore::callback, value))?;

                    Ok(true)
                }
            }
        }

        fn update_location(
            &mut self,
            _timestamp: u64,
            _location: Option<(f64, f64)>,
            _speed: Option<(f64, f64)>,
        ) -> bool {
            self.loc = true;
            true
        }
    }
    

    fn handle_event<T>(engine: &mut T, event: serde_json::Value, expected_response: serde_json::Value, _expected_sleep: Option<u64>) {
        let event = serde_json::to_vec(&event).unwrap();
        let event = event.as_slice();

        let mut response = [0u8; 1024];

        // let mut sleep_called = false;
        let result = engine.handle_event(
            event,
            &mut response,
            &mut |_time, _cb| {
                // assert_eq!(time, expected_sleep.unwrap());
                // sleep_called = true;
                Ok(())
            }
        );
        let len = if let Ok(Some(len)) = result {
            len
        } else {
            panic!("Expected Ok(Some(len))");
        };
        let result: serde_json::Value = serde_json::from_slice(&response[..len]).unwrap();
        assert_eq!(result, expected_response);

        // assert_eq!(sleep_called, expected_sleep.is_some());
    }

}