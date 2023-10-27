#[cfg(test)]
mod tests {
    use crate::core::*;
    use crate::race::*;

    fn bump(race: &mut Race, timestamp: u64, seconds: i32, expected_start: u64) {
        let event = Event {
            event: EventType::BumpSeq {
                timestamp: timestamp,
                seconds: seconds,
            },
        };

        let result = race.handle_event(event, &mut |time, _| {
            assert_eq!(time, expected_start);
            Ok(())
        });

        assert_eq!(result, Ok(true));

        if let State::InSequence { start_time, .. } = race.state {
            assert_eq!(start_time, expected_start);
        } else {
            panic!("State was not InSequence as expected");
        }
    }

    #[test]
    fn test_activation() {
        let mut race = Race::default();
        let event = Event {
            event: EventType::Activate,
        };
        let result = race.handle_event(event, &mut |_, _| Ok(()));
        assert_eq!(result, Ok(true));
        if let State::Active { speed } = race.state {
            assert_eq!(speed, 0.0);
        } else {
            panic!("State was not Active as expected");
        }

    }

    #[test]
    fn test_line() {
        let mut race = Race::default();
        let loc1 = (38.3, -134.2);
        let loc2 = (32.3, -113.2);

        let speed = (18.0, 358.1);

        // set a location for stbd
        let result = race.update_location(Some(loc1), Some(speed));
        assert_eq!(result, true);

        let event = Event {
            event: EventType::LineStbd,
        };
        let result = race.handle_event(event, &mut |_, _| Ok(()));
        assert_eq!(result, Ok(true));
        if let Line::Stbd{location} = race.line {
            assert_eq!(location.lat, loc1.0);
            assert_eq!(location.lon, loc1.1);
        } else {
            panic!("Line was not Stbd as expected");
        }

        // set a new location for stbd
        let result = race.update_location(Some(loc2), Some(speed));
        assert_eq!(result, true);
        let event = Event {
            event: EventType::LineStbd,
        };
        let result = race.handle_event(event, &mut |_, _| Ok(()));
        assert_eq!(result, Ok(true));
        if let Line::Stbd{location} = race.line {
            assert_eq!(location.lat, loc2.0);
            assert_eq!(location.lon, loc2.1);
        } else {
            panic!("Line was not Stbd as expected");
        }

        // set port and check that line is Both
        let result = race.update_location(Some(loc1), Some(speed));
        assert_eq!(result, true);
        let event = Event {
            event: EventType::LinePort,
        };
        let result = race.handle_event(event, &mut |_, _| Ok(()));
        assert_eq!(result, Ok(true));
        assert!(matches!(race.line, Line::Both { .. }));
        if let Line::Both{stbd, port, ..} = race.line {
            assert_eq!(stbd.lat, loc2.0);
            assert_eq!(stbd.lon, loc2.1);
            assert_eq!(port.lat, loc1.0);
            assert_eq!(port.lon, loc1.1);
        } else {
            panic!("Line was not Both as expected");
        }

        // set a new location for port
        let loc3 = (42.3, -113.2);
        let result = race.update_location(Some(loc3), Some(speed));
        assert_eq!(result, true);
        let event = Event {
            event: EventType::LinePort,
        };
        let result = race.handle_event(event, &mut |_, _| Ok(()));
        assert_eq!(result, Ok(true));
        if let Line::Both{stbd, port, ..} = race.line {
            assert_eq!(stbd.lat, loc2.0);
            assert_eq!(stbd.lon, loc2.1);
            assert_eq!(port.lat, loc3.0);
            assert_eq!(port.lon, loc3.1);
        } else {
            panic!("Line was not Both as expected");
        }

    }

    #[test]
    fn test_bump_sequence() {
        let mut race = Race::default();
        bump(&mut race, 1000, 30, 31_000);
        // bump up
        bump(&mut race, 25_000, -60, 91_000);
        // bump down
        bump(&mut race, 25_000, 30, 61_000);
        // bump up
        bump(&mut race, 25_000, -300, 361_000);
        // sync to nearest minute
        bump(
            &mut race,
            234_567,
            0,
            361_000 - (361_000 - 234_567) % 60_000,
        );
    }

    #[test]
    fn test_race() {
        let mut race = Race::default();
        bump(&mut race, 1000, 30, 31_000);
        race.start(&());

        if let State::Racing{start_time, speed, heading} = race.state {
            assert_eq!(start_time, 31_000);
            assert_eq!(speed, 0.0);
            assert_eq!(heading, 0.0);
        } else {
            panic!("State was not Racing as expected");
        }

        let event = Event {
            event: EventType::RaceFinish,
        };
        let result = race.handle_event(event, &mut |_, _| Ok(()));
        assert_eq!(result, Ok(true));
        assert!(
            matches!(race.state, State::Idle),
            "State was not Idle as expected",
        );
    }
}
