#[cfg(test)]
mod tests {
    use super::*;
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
        race.handle_event(event, &mut |_, _| {});
        assert!(
            matches!(race.state, State::Active { speed: 0.0 }),
            "State was not Active as expected",
        );
    }

    // #[test]
    // fn test_line_stbd() {
    //     let mut race = Race::default();
    //     let event = Event {
    //         event: EventType::LineStbd,
    //     };
    //     race.handle_event(event, &mut |_, _| {});
    //     assert!(matches!(race.line, Line::Stbd { .. }) || matches!(race.line, Line::Both { .. }));
    // }

    // #[test]
    // fn test_line_port() {
    //     let mut race = Race::default();
    //     let event = Event {
    //         event: EventType::LinePort,
    //     };
    //     race.handle_event(event, &mut |_, _| {});
    //     assert!(matches!(race.line, Line::Port { .. }) || matches!(race.line, Line::Both { .. }));
    // }

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

        assert!(
            matches!(
                race.state,
                State::Racing {
                    start_time: 31_000,
                    speed: 0.0,
                    heading: 0.0
                }
            ),
            "State was not Racing as expected",
        );

        let event = Event {
            event: EventType::RaceFinish,
        };
        race.handle_event(event, &mut |_, _| {});
        assert!(
            matches!(race.state, State::Idle),
            "State was not Idle as expected",
        );
    }
}
