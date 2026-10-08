#[cfg(test)]
mod tests {
    extern crate std;

    use crate::TuneSpeed;
    use extreme_traits::{Engine, Outcome, Velocity};
    use serde_json::json;
    use std::vec;

    #[test]
    fn test_initial_state() {
        let tune = TuneSpeed::<100>::default();

        assert_eq!(tune.speed, 0.0);
        assert_eq!(tune.speed_dev, 0.0);
        assert_eq!(tune.heading_dev, 0.0);
    }

    #[test]
    fn test_single_sample() {
        let mut tune = TuneSpeed::<100>::default();

        let timestamp = 0;
        let speed = 10.0;
        let heading = 90.0;

        let result = tune.location_event(timestamp, None, Some(Velocity { speed, heading }));

        assert_eq!(result, Outcome::CHANGED);
        assert_eq!(tune.speed, speed);
        assert_eq!(tune.speed_dev, 0.0);
        assert_eq!(tune.heading_dev, 0.0);
    }

    #[test]
    fn test_multiple_samples() {
        let mut tune = TuneSpeed::<100>::default();

        // Simulate receiving speed and heading data at irregular intervals
        let samples = vec![
            (0_u64, 10.0, 90.0),
            (5000, 11.0, 92.0),
            (15000, 9.0, 88.0),
            (25000, 12.0, 91.0),
            (35000, 13.0, 93.0),
        ];

        for (timestamp, speed, heading) in samples.iter() {
            let _ = tune.location_event(
                *timestamp,
                None,
                Some(Velocity {
                    speed: *speed,
                    heading: *heading,
                }),
            );
        }

        // Calculate expected weighted average speed over the last 30 seconds
        // The window is from 5000 to 35000 milliseconds

        let weighted_speeds = vec![
            (11.0, 10000_u64), // From 5000 to 15000 ms
            (9.0, 10000_u64),  // From 15000 to 25000 ms
            (12.0, 10000_u64), // From 25000 to 35000 ms
        ];

        let total_time = weighted_speeds
            .iter()
            .map(|&(_, dt)| dt as f64)
            .sum::<f64>();

        let expected_speed = weighted_speeds
            .iter()
            .map(|&(speed, dt)| speed * dt as f64)
            .sum::<f64>()
            / total_time;

        let current_speed = 13.0;
        let expected_speed_dev = current_speed - expected_speed;

        // `speed` reports the current speed; the 30s average only feeds `speed_dev`
        assert!((tune.speed - current_speed).abs() < 0.001);
        assert!((tune.speed_dev - expected_speed_dev).abs() < 0.001);

        // For heading, similar calculation using circular statistics
        // We'll compute the weighted average heading
        let weighted_headings = vec![
            (92.0_f64.to_radians(), 10000_u64), // From 5000 to 15000 ms
            (88.0_f64.to_radians(), 10000_u64), // From 15000 to 25000 ms
            (91.0_f64.to_radians(), 10000_u64), // From 25000 to 35000 ms
        ];

        let sum_sin = weighted_headings
            .iter()
            .map(|&(heading_rad, dt)| heading_rad.sin() * dt as f64)
            .sum::<f64>();

        let sum_cos = weighted_headings
            .iter()
            .map(|&(heading_rad, dt)| heading_rad.cos() * dt as f64)
            .sum::<f64>();

        let avg_heading_rad = sum_sin.atan2(sum_cos);
        let avg_heading_deg = avg_heading_rad.to_degrees();

        let current_heading = 93.0;
        let mut expected_heading_dev = current_heading - avg_heading_deg;

        // Normalize heading deviation to [-180, 180]
        expected_heading_dev = ((expected_heading_dev + 180.0) % 360.0) - 180.0;

        assert!((tune.heading_dev - expected_heading_dev).abs() < 0.001);
    }

    #[test]
    fn test_compact_state() {
        let mut tune = TuneSpeed::<100>::default();
        let _ = tune.location_event(
            0,
            None,
            Some(Velocity {
                speed: 10.0,
                heading: 90.0,
            }),
        );
        let _ = tune.location_event(
            1000,
            None,
            Some(Velocity {
                speed: 12.0,
                heading: 95.0,
            }),
        );

        let mut buf = [0u8; 8];
        assert_eq!(tune.compact_state(1000, &mut buf), 6);
        assert_eq!(u16::from_le_bytes([buf[0], buf[1]]), 1200);
        assert_eq!(
            i16::from_le_bytes([buf[2], buf[3]]),
            (tune.speed_dev * 100.0) as i16
        );
        assert_eq!(
            i16::from_le_bytes([buf[4], buf[5]]),
            (tune.heading_dev * 10.0) as i16
        );
        assert_eq!(tune.compact_state(1000, &mut buf[..5]), 0);
    }

    #[test]
    fn test_serialization() {
        let mut tune = TuneSpeed::<100>::default();

        let _ = tune.location_event(0, None, Some(vel(10.0, 90.0)));
        let _ = tune.location_event(1000, None, Some(vel(12.0, 95.0)));

        let serialized = extreme_traits::serialize_state(&tune).unwrap();
        let expected_json = json!({
            "speed": tune.speed,
            "speed_dev": tune.speed_dev,
            "heading_dev": tune.heading_dev,
        });

        let parsed_json: serde_json::Value = serde_json::from_slice(&serialized).unwrap();

        assert_eq!(parsed_json, expected_json);
    }

    #[test]
    fn test_out_of_order_samples_are_ignored() {
        let mut tune = TuneSpeed::<100>::default();

        assert_eq!(
            tune.location_event(5000, None, Some(vel(10.0, 90.0))),
            Outcome::CHANGED
        );
        assert_eq!(
            tune.location_event(6000, None, Some(vel(12.0, 95.0))),
            Outcome::CHANGED
        );
        let (speed, speed_dev, heading_dev) = (tune.speed, tune.speed_dev, tune.heading_dev);

        // older and repeated timestamps must not panic or change anything
        assert_eq!(
            tune.location_event(1000, None, Some(vel(20.0, 180.0))),
            Outcome::NONE
        );
        assert_eq!(
            tune.location_event(6000, None, Some(vel(20.0, 180.0))),
            Outcome::NONE
        );
        assert_eq!(
            (tune.speed, tune.speed_dev, tune.heading_dev),
            (speed, speed_dev, heading_dev)
        );

        // no velocity, no change
        assert_eq!(tune.location_event(7000, None, None), Outcome::NONE);
    }

    fn vel(speed: f64, heading: f64) -> Velocity {
        Velocity { speed, heading }
    }
}
