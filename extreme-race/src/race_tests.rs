
mod tests {
    use crate::race::*;
    use crate::line::Line;
    use core::f64::consts::PI;
    use extreme_traits::Engine;
    use serde_json;
    use serde_json::json;


    #[test]
    fn test_sequence() {
        let mut race = Race::default();

        //
        // State: Active
        //
        assert_json_eq(json!({
            "state": "Active",
            "speed": 0.0,
            "line": "None",
        }), race);

        assert_eq!(
            race.location_event(0, None, Some((23.2, 350.0))),
            (None, None),
        );

        assert_json_eq(json!({
            "state": "Active",
            "speed": 23.2,
            "line": "None",
        }), race);

        // set stbd pin
        assert_eq!(
            race.external_event(
                0, 
                Event::LineStbd,
            ),
            (Some(()), None),
        );

        assert_json_eq(json!({
            "state": "Active",
            "speed": 23.2,
            "line": "Stbd",
        }), race);


        //
        // State: InSequence
        //
        bump(&mut race, 1000, 30, 31_000);

        assert_json_eq(json!({
            "state": "InSequence",
            "speed": 23.2,
            "start_time": 31_000,
            "line": "Stbd",
        }), race);


        // set port pin
        assert_eq!(
            race.external_event(
                0, 
                Event::LinePort,
            ),
            (Some(()), None),
        );

        assert_json_eq(json!({
            "state": "InSequence",
            "speed": 23.2,
            "start_time": 31_000,
            "line": "Both",
            "line_cross": 0,
            "line_timestamp": 0,
        }), race);


        //
        // State: Racing
        //
        assert_eq!(
            race.timer_event(31_000),
            (Some(()), None),
        );

        assert_json_eq(json!({
            "state": "Racing",
            "speed": 23.2,
            // we don't keep speed across the state change
            "heading": 0.0, 
            "start_time": 31_000,
        }), race);


        //
        // State: Active
        //
        assert_eq!(
            race.external_event(
                0, 
                Event::RaceFinish,
            ),
            (Some(()), None),
        );

        assert_json_eq(json!({
            "state": "Active",
            "speed": 23.2,
            "line": "Both",
            "line_cross": 0,
            "line_timestamp": 0,
        }), race);

    }

    #[test]
    fn test_line() {
        let mut race = Race::default();
        let loc1 = (38.3, -134.2);
        let loc2 = (32.3, -113.2);

        // set a location for stbd
        assert_eq!(race.location_event(0, Some(loc1), None), (None, None));

        assert_eq!(
            race.external_event(0, Event::LineStbd),
            (Some(()), None)
        );

        if let Line::Stbd { stbd_location } = race.line {
            assert_eq!(stbd_location.lat, to_rad(loc1.0));
            assert_eq!(stbd_location.lon, to_rad(loc1.1));
        } else {
            panic!("Line was not Stbd as expected");
        }

        // set a new location for stbd
        let (updated, timer) = race.location_event(0, Some(loc2), None);
        assert_eq!(None, updated);
        assert_eq!(None, timer);

        assert_eq!(
            race.external_event(0, Event::LineStbd),
            (None, None)
        );
        if let Line::Stbd { stbd_location } = race.line {
            assert_eq!(stbd_location.lat, to_rad(loc2.0));
            assert_eq!(stbd_location.lon, to_rad(loc2.1));
        } else {
            panic!("Line was not Stbd as expected");
        }

        // set port and check that line is Both
        assert_eq!(
            race.location_event(0, Some(loc1), None),
            (None, None)
        );
        assert_eq!(
            race.external_event(0, Event::LinePort),
            (Some(()), None)
        );
        assert!(matches!(race.line, Line::Both { .. }));
        if let Line::Both { stbd, port, .. } = race.line {
            assert_eq!(stbd.lat, to_rad(loc2.0));
            assert_eq!(stbd.lon, to_rad(loc2.1));
            assert_eq!(port.lat, to_rad(loc1.0));
            assert_eq!(port.lon, to_rad(loc1.1));
        } else {
            panic!("Line was not Both as expected");
        }

        // set a new location for port
        let loc3 = (42.3, -113.2);
        assert_eq!(
            race.location_event(0, Some(loc3), None),
            (None, None)
        );
        assert_eq!(
            race.external_event(0, Event::LinePort),
            (Some(()), None)
        );

        if let Line::Both { stbd, port, .. } = race.line {
            assert_eq!(stbd.lat, to_rad(loc2.0));
            assert_eq!(stbd.lon, to_rad(loc2.1));
            assert_eq!(port.lat, to_rad(loc3.0));
            assert_eq!(port.lon, to_rad(loc3.1));
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
        assert_eq!(
            race.timer_event(31_000),
            (Some(()), None)
        );

        if let State::Racing {
            start_time,
            speed,
            heading,
        } = race.state
        {
            assert_eq!(start_time, 31_000);
            assert_eq!(speed, 0.0);
            assert_eq!(heading, 0.0);
        } else {
            panic!("State was not Racing as expected");
        }

        assert_eq!(
            race.external_event(0, Event::RaceFinish),
            (Some(()), None)
        );
        assert!(
            matches!(race.state, State::Active { .. }),
            "State was not Active as expected",
        );
    }

    #[test]
    fn test_line_cross() {
        let mut race = Race::default();

        let stbd = (-34.956404, 138.503427);
        let boat_loc = (-34.956800, 138.504157);
        let port = (-34.957152, 138.503438);

        set_line(&mut race, &stbd, &port);

        let boat_velocity = (5.0, 270.0);

        expect_cross(&mut race, &boat_loc, &boat_velocity, 47, 25657);

        // about middle
        let boat_velocity = (10.0, 270.0);
        expect_cross(&mut race, &boat_loc, &boat_velocity, 47, 12828);

        // away
        let boat_velocity = (10.0, 90.0);
        expect_cross(&mut race, &boat_loc, &boat_velocity, 0, 14836);

        // stbd end
        let boat_velocity = (10.0, 250.0);
        expect_cross(&mut race, &boat_loc, &boat_velocity, 18, 13592);

        let boat_velocity = (10.0, 230.0);
        expect_cross(&mut race, &boat_loc, &boat_velocity, 0, 14836);

        // port end
        let boat_velocity = (10.0, 290.0);
        expect_cross(&mut race, &boat_loc, &boat_velocity, 76, 13712);

        // let boat_velocity = (10.0, 310.0);
        // expect_cross(&mut race, &boat_loc, &boat_velocity, 100, 13712);
    }

    #[test]
    fn test_json() {
        let mut race = Race::default();

        assert_eq!(
            race.location_event(0, Some((42.3, -113.2)), Some((12.5, 270.0))),
            (Some(()), None),
        );

        assert_json_eq(json!({
            "state": "Active",
            "speed": 12.5,
            "line": "None"
        }), race);

        
        assert_eq!(
            race.external_event(
                0, 
                Event::LineStbd,
            ),
            (Some(()), None),
        );

        assert_json_eq(json!({
            "state": "Active",
            "speed": 12.5,
            "line": "Stbd",
        }), race);

        assert_eq!(
            race.external_event(
                0, 
                Event::LinePort,
            ),
            (Some(()), None),
        );

        assert_json_eq(json!({
            "state": "Active",
            "speed": 12.5,
            "line": "Both",
            "line_cross": 0,
            "line_timestamp": 0,
        }), race);
    }


    fn assert_json_eq<Actual: serde::Serialize>(expected: serde_json::Value, actual: Actual) {
        let json_result = serde_json::to_string(&actual).unwrap();
        // println!("{}", actual);

        let expected_json = serde_json::json!(expected);        
    
        let actual: serde_json::Value = serde_json::from_str(&json_result).expect("Invalid JSON");

        assert_eq!(expected_json, actual);

    }


    // #[test]
    // fn field_test() {
    //     let mut race = Race::default();
    //     const START_TIME: u64 = 1000;

    //     bump(&mut race, START_TIME, 30, 31_000);
    //     race.start(&());

    //     fn to_deg(rad: f64) -> f64 {
    //         rad * 180.0 / PI
    //     }

    //     let stbd = (to_deg(-0.609884915845991), to_deg(2.4193028615952987));
    //     let port = (to_deg(-0.6098849332992835), to_deg(2.419303597542466));
    //     set_line(&mut race, &stbd, &port);

    //     let boat_velocity = (1.405, 190.0);

    //     let boat_loc = (to_deg(-0.6098844620603855), to_deg(2.4193031292124503));
    //     expect_cross(&mut race, &boat_loc, &boat_velocity, 77, 4096);

    //     let boat_loc = (to_deg(-0.6098844720603855), to_deg(2.4193031292124503));
    //     expect_cross(&mut race, &boat_loc, &boat_velocity, 76, 4007);

    //     let boat_loc = (to_deg(-0.6098848620603855), to_deg(2.4193031292124503));
    //     expect_cross(&mut race, &boat_loc, &boat_velocity, 65, 534);

    //     // cross
    //     let boat_loc = (to_deg(-0.6098849220603855), to_deg(2.4193031292124503));
    //     expect_cross(&mut race, &boat_loc, &boat_velocity, 63, 0);

    //     let boat_loc = (to_deg(-0.6098849320603855), to_deg(2.4193031292124503));
    //     expect_cross(&mut race, &boat_loc, &boat_velocity, 100, 1938);
    // }

    fn to_rad(deg: f64) -> f64 {
        deg * PI / 180.0
    }

    fn set_line(race: &mut Race, stbd: &(f64, f64), port: &(f64, f64)) {
        //
        // set a location for stbd
        //
        assert_eq!(
            race.location_event(0, Some(*stbd), None), 
            (None, None)
        );

        assert_eq!(
            race.external_event(0, Event::LineStbd),
            (Some(()), None)
        );
        assert!(matches!(race.line, Line::Stbd { .. }));

        //
        // set a new location for port
        //
        assert_eq!(
            race.location_event(0, Some(*port), None),
            (None, None)
        );

        assert_eq!(
            race.external_event(0, Event::LinePort),
            (Some(()), None)
        );
        assert!(matches!(race.line, Line::Both { .. }));
    }

    fn expect_cross(
        race: &mut Race,
        boat_loc: &(f64, f64),
        boat_velocity: &(f64, f64),
        expected_cross: u8,
        expected_timestamp: u64,
    ) {
        let (updated, timer) = race.location_event(0, Some(*boat_loc), Some(*boat_velocity));
        assert_eq!(Some(()), updated);
        assert_eq!(None, timer);


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
            race.external_event(
                0, 
                Event::BumpSeq {
                    timestamp: timestamp,
                    seconds: seconds,
                },
            ),
            (Some(()), Some(expected_start)),
        );

        if let State::InSequence { start_time, .. } = race.state {
            assert_eq!(start_time, expected_start);
        } else {
            panic!("State was not InSequence as expected");
        }
    }

}

