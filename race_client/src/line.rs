use ::serde::ser::SerializeStruct;
use ::serde::Serializer;

use engine::FlatDiffSer;
use crate::geo_math::{bearing, distance, seconds_to_line};
use crate::types::Location;

const R: f64 = 6371e3; // radius of earth in meters

#[derive(FlatDiffSer, Copy, Clone, PartialEq, Default)]
pub enum Line {
    #[default]
    None,

    #[delta(skip_fields)]
    Stbd { stbd_location: Location },

    #[delta(skip_fields)]
    Port { port_location: Location },

    Both {
        line_timestamp: u64,
        line_cross: u8,

        #[delta(skip)]
        stbd: Location,

        #[delta(skip)]
        port: Location,

        bearing: f64,
        length: f64,
    },
}

impl Line {
    pub fn set_stbd(&mut self, location: Location) {
        match self {
            Line::None => {
                *self = Line::Stbd {
                    stbd_location: location,
                };
            }
            Line::Stbd { stbd_location: loc } => {
                *loc = location;
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
            }
        }
    }

    pub fn set_port(&mut self, location: Location) {
        match self {
            Line::None => {
                *self = Line::Port {
                    port_location: location,
                };
            }
            Line::Port { port_location: loc } => {
                *loc = location;
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
            }
        }
    }

    pub fn update_location(
        &mut self,
        timestamp: u64,
        location: (f64, f64),
        heading: f64,
        speed: f64,
    ) {
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
            }
            _ => {}
        }
    }
}
