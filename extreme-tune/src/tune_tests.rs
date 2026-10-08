#[cfg(test)]
mod tests {
    extern crate std;

    use crate::{TuneSpeed, TuneView};
    use extreme_sdk::{Engine, Location, Velocity, abi::IO_CAP};
    use std::{vec, vec::Vec};

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

        let result = gps(&mut tune, timestamp, Some(Velocity { speed, heading }));

        assert!(result);
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
            let _ = gps(
                &mut tune,
                *timestamp,
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
    fn test_view_and_postcard() {
        let mut tune = TuneSpeed::<100>::default();

        let _ = gps(&mut tune, 0, Some(vel(10.0, 90.0)));
        let _ = gps(&mut tune, 1000, Some(vel(12.0, 95.0)));

        let view = tune.view();
        assert_eq!(view.speed, 12.0);
        assert_eq!(view.speed_dev, tune.speed_dev as f32);
        assert_eq!(view.heading_dev, tune.heading_dev as f32);

        // three little-endian f32s, nothing else
        let bytes = state_bytes(&tune);
        assert_eq!(bytes.len(), 12);
        assert_eq!(&bytes[..4], &12.0f32.to_le_bytes());
        assert_eq!(postcard::from_bytes::<TuneView>(&bytes).unwrap(), view);
        assert!(bytes.len() <= IO_CAP);

        // no events: `()` is no bytes at all, and changes nothing
        assert_eq!(extreme_sdk::raw_event(&mut tune, 0, &[]), Some(false));
        assert_eq!(extreme_sdk::raw_event(&mut tune, 0, &[0]), None);
        // and no timer
        assert_eq!(tune.next_timer(), None);
    }

    #[test]
    fn test_out_of_order_samples_are_ignored() {
        let mut tune = TuneSpeed::<100>::default();

        assert!(gps(&mut tune, 5000, Some(vel(10.0, 90.0))));
        assert!(gps(&mut tune, 6000, Some(vel(12.0, 95.0))));
        let (speed, speed_dev, heading_dev) = (tune.speed, tune.speed_dev, tune.heading_dev);

        // older and repeated timestamps must not panic or change anything
        assert!(!gps(&mut tune, 1000, Some(vel(20.0, 180.0))));
        assert!(!gps(&mut tune, 6000, Some(vel(20.0, 180.0))));
        assert_eq!(
            (tune.speed, tune.speed_dev, tune.heading_dev),
            (speed, speed_dev, heading_dev)
        );

        // no velocity, no change
        assert!(!gps(&mut tune, 7000, None));
    }

    fn vel(speed: f64, heading: f64) -> Velocity {
        Velocity { speed, heading }
    }

    /// A GPS update with no fix (the tuning engine uses none); whether
    /// the view changed.
    fn gps<const N: usize>(tune: &mut TuneSpeed<N>, timestamp: u64, velocity: Option<Velocity>) -> bool {
        tune.on_location(&Location {
            timestamp,
            fix: None,
            velocity,
        })
    }

    /// The engine's view as postcard, as the ABI's `view` writes it.
    fn state_bytes<E: Engine>(engine: &E) -> Vec<u8> {
        let mut out = [0u8; IO_CAP];
        let len = extreme_sdk::encode_view(engine, &mut out).unwrap();
        out[..len].to_vec()
    }
}
