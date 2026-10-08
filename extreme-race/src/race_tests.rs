mod tests {
    extern crate std;

    use crate::line::Line;
    use crate::race::*;
    use core::f64::consts::PI;
    use extreme_traits::{Engine, Fix, MAX_STATE_SIZE, Outcome, StateBytes, Velocity};

    #[test]
    fn test_sequence() {
        let mut race = Race::default();

        //
        // State: Active
        //
        assert_view(&race, 0.0, Phase::Active { line: LineView::None });

        assert_eq!(
            race.location_event(0, None, Some(vel(23.2, 350.0))),
            Outcome::CHANGED,
        );

        assert_view(&race, 23.2, Phase::Active { line: LineView::None });

        // set stbd pin
        assert_eq!(
            race.external_event(0, &RaceEvent::LineStbd),
            Outcome::CHANGED,
        );

        assert_view(&race, 23.2, Phase::Active { line: LineView::Stbd });

        //
        // State: InSequence
        //
        bump(&mut race, 1000, 30, 31_000);

        assert_view(
            &race,
            23.2,
            Phase::InSequence {
                start_time: 31_000,
                line: LineView::Stbd,
            },
        );

        // set port pin
        assert_eq!(
            race.external_event(0, &RaceEvent::LinePort),
            Outcome::CHANGED,
        );

        assert_view(
            &race,
            23.2,
            Phase::InSequence {
                start_time: 31_000,
                line: LineView::Both { cross: 0, time: 0 },
            },
        );

        //
        // State: Racing
        //
        assert_eq!(race.timer_event(31_000), Outcome::CHANGED);

        // we don't keep heading across the state change, and the line is
        // no longer shown
        assert_view(
            &race,
            23.2,
            Phase::Racing {
                start_time: 31_000,
                heading: 0.0,
            },
        );

        assert_eq!(
            race.location_event(0, None, Some(vel(17.5, 253.0))),
            Outcome::CHANGED,
        );

        assert_view(
            &race,
            17.5,
            Phase::Racing {
                start_time: 31_000,
                heading: 253.0,
            },
        );

        //
        // State: Active
        //
        assert_eq!(
            race.external_event(0, &RaceEvent::RaceFinish),
            Outcome::CHANGED.cancel_timer(),
        );

        assert_view(
            &race,
            17.5,
            Phase::Active {
                line: LineView::Both { cross: 0, time: 0 },
            },
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
            race.external_event(0, &RaceEvent::LineStbd),
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
            race.external_event(0, &RaceEvent::LineStbd),
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
            race.external_event(0, &RaceEvent::LinePort),
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
            race.external_event(0, &RaceEvent::LinePort),
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

    /// `ago` back-dates the tap: a sync confirmed 1.5 s after the tap
    /// rounds from the tap.
    #[test]
    fn test_bump_ago() {
        let mut race = Race::default();
        // tapped at 3500, arrived at 5000: the gun is 30 s from the tap
        assert_eq!(
            race.external_event(5000, &RaceEvent::BumpSeq { seconds: 30, ago: 1500 }),
            Outcome::CHANGED.with_timer(33_500)
        );
        assert!(matches!(
            race.state,
            State::InSequence { start_time: 33_500 }
        ));
        // tapped at 3600 (29.9 s left), confirmed at 5100 (28.4 s left):
        // the 29.9 s left round down to 0: the start moves to the tap
        assert_eq!(
            race.external_event(5100, &RaceEvent::BumpSeq { seconds: 0, ago: 1500 }),
            Outcome::CHANGED.with_timer(3600)
        );
        // an `ago` from before the epoch clamps
        let mut race = Race::default();
        assert_eq!(
            race.external_event(100, &RaceEvent::BumpSeq { seconds: 60, ago: 1000 }),
            Outcome::CHANGED.with_timer(60_000)
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
            race.external_event(0, &RaceEvent::RaceFinish),
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
            race.external_event(5_000, &RaceEvent::RaceFinish),
            Outcome::CHANGED.cancel_timer()
        );
        assert!(matches!(race.state, State::Active));

        // a timer that fires anyway (raced with the cancel) must be ignored
        assert_eq!(race.timer_event(31_000), Outcome::NONE);
        assert!(matches!(race.state, State::Active));
    }

    /// The view on the wire (postcard): field order, enum tags, varints,
    /// little-endian f32.
    #[test]
    fn test_postcard_state() {
        let mut race = Race::default();
        let _ = race.location_event(1000, None, Some(vel(6.4, 90.0)));
        // speed 6.4f32 = 0x40CCCCCD, Active = 0, line None = 0
        assert_eq!(
            &state_bytes(&race)[..],
            &[0xcd, 0xcc, 0xcc, 0x40, 0, 0]
        );

        // InSequence = 1, start_time 31 000 = varint 98 f2 01, Stbd = 1
        bump(&mut race, 1000, 30, 31_000);
        let _ = race.external_event(0, &RaceEvent::LineStbd);
        assert_eq!(
            &state_bytes(&race)[..],
            &[0xcd, 0xcc, 0xcc, 0x40, 1, 0x98, 0xf2, 0x01, 1]
        );

        // Racing = 2, then the heading, 0.0
        assert_eq!(race.timer_event(31_000), Outcome::CHANGED);
        assert_eq!(
            &state_bytes(&race)[..],
            &[0xcd, 0xcc, 0xcc, 0x40, 2, 0x98, 0xf2, 0x01, 0, 0, 0, 0]
        );

        let view: RaceView = postcard::from_bytes(&state_bytes(&race)).unwrap();
        assert_eq!(view, race.view());
    }

    #[test]
    fn test_postcard_events() {
        let mut race = Race::default();
        let _ = race.location_event(0, Some(fix(38.3, -134.2)), None);

        assert_eq!(postcard::to_stdvec(&RaceEvent::LineStbd).unwrap(), [0]);
        assert_eq!(extreme_traits::RawEngine::external_event(&mut race, 0, &[0]), Ok(Outcome::CHANGED));
        assert!(matches!(race.line, Line::Stbd { .. }));

        // bump +30 s (zigzag 60), tapped 1500 ms (dc 0b) before the event
        // arrived at t = 5000
        let bump = postcard::to_stdvec(&RaceEvent::BumpSeq { seconds: 30, ago: 1500 }).unwrap();
        assert_eq!(bump, [2, 60, 0xdc, 0x0b]);
        assert_eq!(
            extreme_traits::RawEngine::external_event(&mut race, 5000, &bump),
            Ok(Outcome::CHANGED.with_timer(3500 + 30_000))
        );
        assert!(matches!(
            race.state,
            State::InSequence { start_time: 33_500 }
        ));
        // -300 s: zigzag 599
        assert_eq!(
            postcard::to_stdvec(&RaceEvent::BumpSeq { seconds: -300, ago: 0 }).unwrap(),
            [2, 0xd7, 0x04, 0]
        );

        assert_eq!(
            extreme_traits::RawEngine::external_event(&mut race, 0, &[3]),
            Ok(Outcome::CHANGED.cancel_timer())
        );
        assert!(matches!(race.state, State::Active));

        // truncated, unknown, empty, trailing
        assert_eq!(extreme_traits::RawEngine::external_event(&mut race, 0, &[2, 1]), Err(()));
        assert_eq!(extreme_traits::RawEngine::external_event(&mut race, 0, &[4]), Err(()));
        assert_eq!(extreme_traits::RawEngine::external_event(&mut race, 0, &[]), Err(()));
        assert_eq!(extreme_traits::RawEngine::external_event(&mut race, 0, &[3, 0]), Err(()));
    }

    /// The serialized view of each phase at its largest: start and line
    /// times at today's epoch milliseconds take 6 bytes as varints (up to
    /// 2^42 ms, the year 2109), 10 at the most (u64::MAX).
    #[test]
    fn test_state_sizes() {
        let now: u64 = 1_790_000_000_000;
        let size = |phase| {
            let view = RaceView { speed: 12.3, phase };
            postcard::to_stdvec(&view).unwrap().len()
        };
        let both = |time| LineView::Both { cross: 100, time };
        assert_eq!(size(Phase::Active { line: LineView::None }), 6);
        assert_eq!(size(Phase::Active { line: both(now) }), 13);
        assert_eq!(size(Phase::InSequence { start_time: now, line: LineView::Port }), 12);
        assert_eq!(size(Phase::InSequence { start_time: now, line: both(now) }), 19);
        assert_eq!(size(Phase::Racing { start_time: now, heading: 359.9 }), 15);
        // the worst case, plus the engine tag, fits the state buffer
        let worst = size(Phase::InSequence {
            start_time: u64::MAX,
            line: both(u64::MAX),
        });
        assert_eq!(worst, 27);
        assert!(1 + worst <= MAX_STATE_SIZE);
        let mut race = Race::default();
        race.state = State::InSequence { start_time: u64::MAX };
        race.line = Line::Both {
            line_timestamp: u64::MAX,
            line_cross: 100,
            stbd: Default::default(),
            port: Default::default(),
            length: 0.0,
        };
        assert_eq!(state_bytes(&race).len(), worst);
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

        // and the view shows it
        assert_eq!(
            race.view().phase,
            Phase::Active {
                line: LineView::Both {
                    cross: 76,
                    time: 13712
                }
            }
        );
    }

    #[test]
    fn test_view_line() {
        let mut race = Race::default();

        assert_eq!(
            race.location_event(0, Some(fix(42.3, -113.2)), Some(vel(12.5, 270.0))),
            Outcome::CHANGED,
        );
        assert_view(&race, 12.5, Phase::Active { line: LineView::None });

        assert_eq!(
            race.external_event(0, &RaceEvent::LineStbd),
            Outcome::CHANGED,
        );
        assert_view(&race, 12.5, Phase::Active { line: LineView::Stbd });

        assert_eq!(
            race.external_event(0, &RaceEvent::LinePort),
            Outcome::CHANGED,
        );
        assert_view(
            &race,
            12.5,
            Phase::Active {
                line: LineView::Both { cross: 0, time: 0 },
            },
        );
    }

    fn assert_view(race: &Race, speed: f32, phase: Phase) {
        assert_eq!(race.view(), RaceView { speed, phase });
        // and it survives the wire
        let bytes = state_bytes(race);
        assert_eq!(postcard::from_bytes::<RaceView>(&bytes).unwrap(), race.view());
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
            race.external_event(0, &RaceEvent::LineStbd),
            Outcome::CHANGED
        );
        assert!(matches!(race.line, Line::Stbd { .. }));

        //
        // set a new location for port
        //
        assert_eq!(race.location_event(0, Some(port), None), Outcome::NONE);

        assert_eq!(
            race.external_event(0, &RaceEvent::LinePort),
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

    /// A bump tapped at `timestamp` (arriving then, `ago` 0).
    fn bump(race: &mut Race, timestamp: u64, seconds: i32, expected_start: u64) {
        assert_eq!(
            race.external_event(timestamp, &RaceEvent::BumpSeq { seconds, ago: 0 }),
            Outcome::CHANGED.with_timer(expected_start),
        );

        if let State::InSequence { start_time } = race.state {
            assert_eq!(start_time, expected_start);
        } else {
            panic!("State was not InSequence as expected");
        }
    }

    /// The engine's serialized state ([`extreme_traits::RawEngine`]).
    fn state_bytes<E: Engine>(engine: &E) -> StateBytes {
        extreme_traits::RawEngine::serialize_state(engine).unwrap()
    }
}
