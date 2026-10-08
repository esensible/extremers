mod tests {
    use crate::line::Line;
    use crate::race::*;
    use core::f64::consts::PI;
    use extreme_traits::{Engine, Fix, Outcome, Velocity};
    use serde_json::json;

    #[test]
    fn test_sequence() {
        let mut race = Race::default();

        //
        // State: Active
        //
        assert_json_eq(
            json!({
                "state": "Active",
                "speed": 0.0,
                "line": "None",
            }),
            race,
        );

        assert_eq!(
            race.location_event(0, None, Some(vel(23.2, 350.0))),
            Outcome::CHANGED,
        );

        assert_json_eq(
            json!({
                "state": "Active",
                "speed": 23.2,
                "line": "None",
            }),
            race,
        );

        // set stbd pin
        assert_eq!(
            race.external_event(0, &ev(EventType::LineStbd)),
            Outcome::CHANGED,
        );

        assert_json_eq(
            json!({
                "state": "Active",
                "speed": 23.2,
                "line": "Stbd",
            }),
            race,
        );

        //
        // State: InSequence
        //
        bump(&mut race, 1000, 30, 31_000);

        assert_json_eq(
            json!({
                "state": "InSequence",
                "speed": 23.2,
                "start_time": 31_000,
                "line": "Stbd",
            }),
            race,
        );

        // set port pin
        assert_eq!(
            race.external_event(0, &ev(EventType::LinePort)),
            Outcome::CHANGED,
        );

        assert_json_eq(
            json!({
                "state": "InSequence",
                "speed": 23.2,
                "start_time": 31_000,
                "line": "Both",
                "line_cross": 0,
                "line_timestamp": 0,
            }),
            race,
        );

        //
        // State: Racing
        //
        assert_eq!(race.timer_event(31_000), Outcome::CHANGED);

        assert_json_eq(
            json!({
                "state": "Racing",
                "speed": 23.2,
                // we don't keep heading across the state change
                "heading": 0.0,
                "start_time": 31_000,
            }),
            race,
        );

        assert_eq!(
            race.location_event(0, None, Some(vel(17.5, 253.0))),
            Outcome::CHANGED,
        );

        assert_json_eq(
            json!({
                "state": "Racing",
                "speed": 17.5,
                "heading": 253.0,
                "start_time": 31_000,
            }),
            race,
        );

        //
        // State: Active
        //
        assert_eq!(
            race.external_event(0, &ev(EventType::RaceFinish)),
            Outcome::CHANGED.cancel_timer(),
        );

        assert_json_eq(
            json!({
                "state": "Active",
                "speed": 17.5,
                "line": "Both",
                "line_cross": 0,
                "line_timestamp": 0,
            }),
            race,
        );
    }

    #[test]
    fn test_line() {
        let mut race = Race::default();
        let loc1 = fix(38.3, -134.2);
        let loc2 = fix(32.3, -113.2);

        // set a location for stbd
        assert_eq!(race.location_event(0, Some(loc1), None), Outcome::NONE);

        assert_eq!(
            race.external_event(0, &ev(EventType::LineStbd)),
            Outcome::CHANGED
        );

        if let Line::Stbd { stbd_location } = race.line {
            assert_eq!(stbd_location.lat, to_rad(loc1.lat));
            assert_eq!(stbd_location.lon, to_rad(loc1.lon));
        } else {
            panic!("Line was not Stbd as expected");
        }

        // set a new location for stbd
        assert_eq!(race.location_event(0, Some(loc2), None), Outcome::NONE);

        assert_eq!(
            race.external_event(0, &ev(EventType::LineStbd)),
            Outcome::NONE
        );
        if let Line::Stbd { stbd_location } = race.line {
            assert_eq!(stbd_location.lat, to_rad(loc2.lat));
            assert_eq!(stbd_location.lon, to_rad(loc2.lon));
        } else {
            panic!("Line was not Stbd as expected");
        }

        // set port and check that line is Both
        assert_eq!(race.location_event(0, Some(loc1), None), Outcome::NONE);
        assert_eq!(
            race.external_event(0, &ev(EventType::LinePort)),
            Outcome::CHANGED
        );
        assert!(matches!(race.line, Line::Both { .. }));
        if let Line::Both { stbd, port, .. } = race.line {
            assert_eq!(stbd.lat, to_rad(loc2.lat));
            assert_eq!(stbd.lon, to_rad(loc2.lon));
            assert_eq!(port.lat, to_rad(loc1.lat));
            assert_eq!(port.lon, to_rad(loc1.lon));
        } else {
            panic!("Line was not Both as expected");
        }

        // set a new location for port
        let loc3 = fix(42.3, -113.2);
        assert_eq!(race.location_event(0, Some(loc3), None), Outcome::NONE);
        assert_eq!(
            race.external_event(0, &ev(EventType::LinePort)),
            Outcome::CHANGED
        );

        if let Line::Both { stbd, port, .. } = race.line {
            assert_eq!(stbd.lat, to_rad(loc2.lat));
            assert_eq!(stbd.lon, to_rad(loc2.lon));
            assert_eq!(port.lat, to_rad(loc3.lat));
            assert_eq!(port.lon, to_rad(loc3.lon));
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
    fn test_sync_after_start() {
        let mut race = Race::default();
        bump(&mut race, 1000, 30, 31_000);
        // the start has passed but the timer has not been delivered yet:
        // the start time is left alone and the timer fires immediately
        bump(&mut race, 40_000, 0, 31_000);
        bump(&mut race, 31_000, 0, 31_000);
    }

    #[test]
    fn test_bump_saturates() {
        let mut race = Race::default();
        // starting a sequence that would begin before the epoch
        bump(&mut race, 1000, -30, 0);
        // bumping down past the epoch
        bump(&mut race, 1000, 30, 0);
        // bumping up near u64::MAX
        let mut race = Race::default();
        bump(&mut race, u64::MAX - 1000, 30, u64::MAX);
        bump(&mut race, 0, -30, u64::MAX);
    }

    #[test]
    fn test_race() {
        let mut race = Race::default();
        bump(&mut race, 1000, 30, 31_000);
        assert_eq!(race.timer_event(31_000), Outcome::CHANGED);

        if let State::Racing { start_time } = race.state {
            assert_eq!(start_time, 31_000);
            assert_eq!(race.velocity.speed, 0.0);
            assert_eq!(race.velocity.heading, 0.0);
        } else {
            panic!("State was not Racing as expected");
        }

        assert_eq!(
            race.external_event(0, &ev(EventType::RaceFinish)),
            Outcome::CHANGED.cancel_timer()
        );
        assert!(
            matches!(race.state, State::Active),
            "State was not Active as expected",
        );
    }

    #[test]
    fn test_abort_sequence_does_not_start_race() {
        let mut race = Race::default();
        bump(&mut race, 1000, 30, 31_000);

        // finishing during the sequence aborts it and drops the start timer
        assert_eq!(
            race.external_event(5_000, &ev(EventType::RaceFinish)),
            Outcome::CHANGED.cancel_timer()
        );
        assert!(matches!(race.state, State::Active));

        // a timer that fires anyway (raced with the cancel) must be ignored
        assert_eq!(race.timer_event(31_000), Outcome::NONE);
        assert!(matches!(race.state, State::Active));
    }

    #[test]
    fn test_compact_state() {
        let mut race = Race::default();
        let mut buf = [0u8; 32];

        // active, nothing set, 6.4 kn heading 90.0
        let _ = race.location_event(
            1000,
            None,
            Some(Velocity {
                speed: 6.4,
                heading: 90.0,
            }),
        );
        assert_eq!(race.compact_state(1000, &mut buf), 15);
        assert_eq!(
            &buf[..15],
            &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x02, 0x84, 0x03]
        );

        // in sequence, start in 31 s as seen 1 s after the bump; stbd set
        bump(&mut race, 1000, 30, 31_000);
        let _ = race.external_event(0, &ev(EventType::LineStbd));
        assert_eq!(race.compact_state(2000, &mut buf), 15);
        assert_eq!(buf[0], 1);
        assert_eq!(buf[1], 1);
        assert_eq!(i32::from_le_bytes(buf[3..7].try_into().unwrap()), 29_000);

        // a past start is negative
        assert_eq!(race.timer_event(31_000), Outcome::CHANGED);
        assert_eq!(race.compact_state(40_000, &mut buf), 15);
        assert_eq!(buf[0], 2);
        assert_eq!(i32::from_le_bytes(buf[3..7].try_into().unwrap()), -9_000);

        // too small a buffer writes nothing
        assert_eq!(race.compact_state(0, &mut buf[..14]), 0);
    }

    #[test]
    fn test_compact_events() {
        let mut race = Race::default();
        let _ = race.location_event(
            0,
            Some(Fix {
                lat: 38.3,
                lon: -134.2,
            }),
            None,
        );

        assert_eq!(race.compact_event(0, &[0x10]), Ok(Outcome::CHANGED));
        assert!(matches!(race.line, Line::Stbd { .. }));

        // bump +30 s, tapped 1500 ms before the event arrived at t = 5000
        let seconds: i16 = 30;
        let ago: u16 = 1500;
        let mut e = [0x12u8, 0, 0, 0, 0];
        e[1..3].copy_from_slice(&seconds.to_le_bytes());
        e[3..5].copy_from_slice(&ago.to_le_bytes());
        assert_eq!(
            race.compact_event(5000, &e),
            Ok(Outcome::CHANGED.with_timer(3500 + 30_000))
        );
        assert!(matches!(
            race.state,
            State::InSequence { start_time: 33_500 }
        ));

        assert_eq!(
            race.compact_event(0, &[0x13]),
            Ok(Outcome::CHANGED.cancel_timer())
        );
        assert!(matches!(race.state, State::Active));

        assert_eq!(race.compact_event(0, &[0x12, 1]), Err(()));
        assert_eq!(race.compact_event(0, &[0x42]), Err(()));
        assert_eq!(race.compact_event(0, &[]), Err(()));
    }

    #[test]
    fn test_line_cross() {
        let mut race = Race::default();

        let stbd = fix(-34.956404, 138.503427);
        let boat_loc = fix(-34.956800, 138.504157);
        let port = fix(-34.957152, 138.503438);

        set_line(&mut race, stbd, port);

        expect_cross(&mut race, boat_loc, vel(5.0, 270.0), 47, 25657);

        // about middle
        expect_cross(&mut race, boat_loc, vel(10.0, 270.0), 47, 12828);

        // away
        expect_cross(&mut race, boat_loc, vel(10.0, 90.0), 0, 14836);

        // stbd end
        expect_cross(&mut race, boat_loc, vel(10.0, 250.0), 18, 13592);

        expect_cross(&mut race, boat_loc, vel(10.0, 230.0), 0, 14836);

        // port end
        expect_cross(&mut race, boat_loc, vel(10.0, 290.0), 76, 13712);
    }

    #[test]
    fn test_json() {
        let mut race = Race::default();

        assert_eq!(
            race.location_event(0, Some(fix(42.3, -113.2)), Some(vel(12.5, 270.0))),
            Outcome::CHANGED,
        );

        assert_json_eq(
            json!({
                "state": "Active",
                "speed": 12.5,
                "line": "None"
            }),
            race,
        );

        assert_eq!(
            race.external_event(0, &ev(EventType::LineStbd)),
            Outcome::CHANGED,
        );

        assert_json_eq(
            json!({
                "state": "Active",
                "speed": 12.5,
                "line": "Stbd",
            }),
            race,
        );

        assert_eq!(
            race.external_event(0, &ev(EventType::LinePort)),
            Outcome::CHANGED,
        );

        assert_json_eq(
            json!({
                "state": "Active",
                "speed": 12.5,
                "line": "Both",
                "line_cross": 0,
                "line_timestamp": 0,
            }),
            race,
        );
    }

    fn assert_json_eq<Actual: serde::Serialize>(expected: serde_json::Value, actual: Actual) {
        let json_result = serde_json::to_string(&actual).unwrap();
        let actual: serde_json::Value = serde_json::from_str(&json_result).expect("Invalid JSON");
        assert_eq!(expected, actual);
    }

    fn ev(event: EventType) -> Event {
        Event { event }
    }

    fn fix(lat: f64, lon: f64) -> Fix {
        Fix { lat, lon }
    }

    fn vel(speed: f64, heading: f64) -> Velocity {
        Velocity { speed, heading }
    }

    fn to_rad(deg: f64) -> f64 {
        deg * PI / 180.0
    }

    fn set_line(race: &mut Race, stbd: Fix, port: Fix) {
        //
        // set a location for stbd
        //
        assert_eq!(race.location_event(0, Some(stbd), None), Outcome::NONE);

        assert_eq!(
            race.external_event(0, &ev(EventType::LineStbd)),
            Outcome::CHANGED
        );
        assert!(matches!(race.line, Line::Stbd { .. }));

        //
        // set a new location for port
        //
        assert_eq!(race.location_event(0, Some(port), None), Outcome::NONE);

        assert_eq!(
            race.external_event(0, &ev(EventType::LinePort)),
            Outcome::CHANGED
        );
        assert!(matches!(race.line, Line::Both { .. }));
    }

    fn expect_cross(
        race: &mut Race,
        boat_loc: Fix,
        boat_velocity: Velocity,
        expected_cross: u8,
        expected_timestamp: u64,
    ) {
        assert_eq!(
            race.location_event(0, Some(boat_loc), Some(boat_velocity)),
            Outcome::CHANGED
        );

        if let Line::Both {
            line_cross,
            line_timestamp,
            ..
        } = race.line
        {
            assert_eq!(line_cross, expected_cross);
            assert_eq!(line_timestamp, expected_timestamp);
        } else {
            panic!("Line was not Both as expected");
        }
    }

    fn bump(race: &mut Race, timestamp: u64, seconds: i32, expected_start: u64) {
        assert_eq!(
            race.external_event(0, &ev(EventType::BumpSeq { timestamp, seconds })),
            Outcome::CHANGED.with_timer(expected_start),
        );

        if let State::InSequence { start_time } = race.state {
            assert_eq!(start_time, expected_start);
        } else {
            panic!("State was not InSequence as expected");
        }
    }
}
