#[cfg(test)]
mod tests {
    use super::TuneSpeed;
    use extreme_traits::Engine;
    use serde_json::json;

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

        let result = tune.location_event(timestamp, None, Some((speed, heading)));

        assert_eq!(result, (Some(()), None));
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
            tune.location_event(*timestamp, None, Some((*speed, *heading)));
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

        assert!((tune.speed - expected_speed).abs() < 0.001);
        assert!((tune.speed_dev - expected_speed_dev).abs() < 0.001);

        // For heading, similar calculation using circular statistics
        // We'll compute the weighted average heading
        let weighted_headings = vec![
            (92.0_f64.to_radians(), 10000_u64), // From 5000 to 15000 ms
            (88.0_f64.to_radians(), 10000_u64), // From 15000 to 25000 ms
            (91.0_f64.to_radians(), 10000_u64), // From 25000 to 35000 ms
        ];

        let total_time = weighted_headings
            .iter()
            .map(|&(_, dt)| dt as f64)
            .sum::<f64>();

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
    fn test_serialization() {
        let mut tune = TuneSpeed::<100>::default();

        tune.location_event(0, None, Some((10.0, 90.0)));
        tune.location_event(1000, None, Some((12.0, 95.0)));

        let serialized = serde_json_core::to_string(&tune).unwrap();
        let expected_json = json!({
            "speed": tune.speed,
            "speed_dev": tune.speed_dev,
            "heading_dev": tune.heading_dev,
        });

        let parsed_json: serde_json::Value = serde_json::from_str(&serialized).unwrap();

        assert_eq!(parsed_json, expected_json);
    }

    // Helper function for floating point comparison
    fn approx_eq(a: f64, b: f64, epsilon: f64) -> bool {
        (a - b).abs() < epsilon
    }
}
