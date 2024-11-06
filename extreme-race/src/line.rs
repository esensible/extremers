use crate::geo_math::{bearing, distance, seconds_to_line};
use crate::types::Location;
use serde::Serialize;

const R: f64 = 6371e3; // radius of earth in meters

#[derive(Copy, Clone, PartialEq, Default, Serialize)]
pub enum Line {
    #[default]
    None,

    Stbd {
        #[serde(skip)]
        stbd_location: Location,
    },

    Port {
        #[serde(skip)]
        port_location: Location 
    },

    Both {
        line_timestamp: u64,
        line_cross: u8,

        #[serde(skip)]
        stbd: Location,
        #[serde(skip)]
        port: Location,

        #[serde(skip)]
        bearing: f64,
        #[serde(skip)]
        length: f64,
    },
}

impl Line {
    pub fn set_stbd(&mut self, location: Location) -> Option<()> {
        match self {
            Line::None => {
                *self = Line::Stbd {
                    stbd_location: location,
                };
                return Some(())
            }
            Line::Stbd { stbd_location: loc } => {
                *loc = location;
                return None
            }
            Line::Port { port_location: loc } => {
                *self = Line::Both {
                    line_timestamp: 0,
                    line_cross: 0,
                    stbd: location,
                    port: *loc,
                    bearing: bearing(location.lat, location.lon, loc.lat, loc.lon),
                    length: distance(location.lat, location.lon, loc.lat, loc.lon, R),
                };
                return Some(())
            }
            Line::Both {
                stbd,
                port,
                bearing: line_bearing,
                length,
                ..
            } => {
                *stbd = location;
                *line_bearing = bearing(location.lat, location.lon, port.lat, port.lon);
                *length = distance(location.lat, location.lon, port.lat, port.lon, R);
                return Some(())
            }
        }
    }

    pub fn set_port(&mut self, location: Location) -> Option<()> {
        match self {
            Line::None => {
                *self = Line::Port {
                    port_location: location,
                };
                return Some(())
            }
            Line::Port { port_location: loc } => {
                *loc = location;
                return None
            }
            Line::Stbd { stbd_location: loc } => {
                *self = Line::Both {
                    line_timestamp: 0,
                    line_cross: 0,
                    stbd: *loc,
                    port: location,
                    bearing: bearing(loc.lat, loc.lon, location.lat, location.lon),
                    length: distance(loc.lat, loc.lon, location.lat, location.lon, R),
                };
                return Some(())
            }
            Line::Both {
                stbd,
                port,
                bearing: line_bearing,
                length,
                ..
            } => {
                *port = location;
                *line_bearing = bearing(stbd.lat, stbd.lon, location.lat, location.lon);
                *length = distance(stbd.lat, stbd.lon, location.lat, location.lon, R);

                // no state change, but the values have been updated
                return Some(())
            }
        }
    }

    pub fn update_location(
        &mut self,
        timestamp: u64,
        location: (f64, f64),
        heading: f64,
        speed: f64,
    ) -> Option<()> {
        match self {
            Line::Both {
                line_timestamp,
                line_cross,
                stbd,
                port,
                bearing,
                length,
                ..
            } => {
                let (_on_line, new_point, new_time) = seconds_to_line(
                    location.0, location.1, heading, speed, stbd.lat, stbd.lon, port.lat, port.lon,
                    *bearing, *length, R,
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

                Some(())
            }
            _ => {
                None
            }
        }
    }
}
