use crate::geo_math::{distance, seconds_to_line};
use crate::types::Location;
const R: f64 = 6371e3; // radius of earth in meters

/// The start line as set so far. Clients see it as a
/// [`LineView`](crate::race::LineView).
#[derive(Copy, Clone, PartialEq, Default)]
pub enum Line {
    #[default]
    None,

    Stbd {
        stbd_location: Location,
    },

    Port {
        port_location: Location,
    },

    Both {
        /// When the boat reaches the line on its current course, epoch ms.
        line_timestamp: u64,
        /// Where along the line, 0-100 from the port end.
        line_cross: u8,

        stbd: Location,
        port: Location,
        length: f64,
    },
}

impl Line {
    pub fn set_stbd(&mut self, location: Location) -> bool {
        match self {
            Line::None => {
                *self = Line::Stbd {
                    stbd_location: location,
                };
                true
            }
            Line::Stbd { stbd_location: loc } => {
                *loc = location;
                false
            }
            Line::Port { port_location: loc } => {
                *self = Line::Both {
                    line_timestamp: 0,
                    line_cross: 0,
                    stbd: location,
                    port: *loc,
                    length: distance(location.lat, location.lon, loc.lat, loc.lon, R),
                };
                true
            }
            Line::Both {
                stbd, port, length, ..
            } => {
                *stbd = location;
                *length = distance(location.lat, location.lon, port.lat, port.lon, R);
                true
            }
        }
    }

    pub fn set_port(&mut self, location: Location) -> bool {
        match self {
            Line::None => {
                *self = Line::Port {
                    port_location: location,
                };
                true
            }
            Line::Port { port_location: loc } => {
                *loc = location;
                false
            }
            Line::Stbd { stbd_location: loc } => {
                *self = Line::Both {
                    line_timestamp: 0,
                    line_cross: 0,
                    stbd: *loc,
                    port: location,
                    length: distance(loc.lat, loc.lon, location.lat, location.lon, R),
                };
                true
            }
            Line::Both {
                stbd, port, length, ..
            } => {
                *port = location;
                *length = distance(stbd.lat, stbd.lon, location.lat, location.lon, R);

                // no state change, but the values have been updated
                true
            }
        }
    }

    pub fn update_location(
        &mut self,
        timestamp: u64,
        location: Location,
        heading: f64,
        speed: f64,
    ) -> bool {
        match self {
            Line::Both {
                line_timestamp,
                line_cross,
                stbd,
                port,
                length,
                ..
            } => {
                let (_on_line, new_point, new_time) = seconds_to_line(
                    location.lat,
                    location.lon,
                    heading,
                    speed,
                    stbd.lat,
                    stbd.lon,
                    port.lat,
                    port.lon,
                    *length,
                    R,
                );

                let abs_new_time = libm::fabs(new_time * 1000.0) as u64;
                let tmp = if new_time < 0.0 {
                    timestamp.checked_sub(abs_new_time)
                } else {
                    timestamp.checked_add(abs_new_time)
                };
                if let Some(ts) = tmp {
                    *line_timestamp = ts;
                }
                *line_cross = (new_point * 100.0) as u8;

                true
            }
            _ => false,
        }
    }
}
