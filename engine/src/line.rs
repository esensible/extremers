use ::serde::ser::SerializeStruct;
use ::serde::Serializer;

use crate::core::FlatDiffSer;
use crate::types::Location;

#[derive(FlatDiffSer, Copy, Clone, PartialEq, Default)]
pub enum Line {
    #[default]
    None,

    // #[delta(skip_fields)]
    Stbd {
        location: Location,
    },

    // #[delta(skip_fields)]
    Port {
        location: Location,
    },

    Both {
        time: f64,
        point: u8,

        // #[delta(skip)]
        stbd: Location,

        // #[delta(skip)]
        port: Location,
    },
}

impl Line {
    pub fn set_stbd(&mut self, location: Location) {
        match self {
            Line::Stbd { location: loc } => {
                *loc = location;
            }
            Line::Port { location: loc } => {
                *self = Line::Both {
                    time: 0.0,
                    point: 0,
                    stbd: location,
                    port: *loc,
                };
            }
            Line::Both { stbd, .. } => {
                *stbd = location;
            }
            _ => {}
        }
    }

    pub fn set_port(&mut self, location: Location) {
        match self {
            Line::Port { location: loc } => {
                *loc = location;
            }
            Line::Stbd { location: loc } => {
                *self = Line::Both {
                    time: 0.0,
                    point: 0,
                    stbd: *loc,
                    port: location,
                };
            }
            Line::Both { port, .. } => {
                *port = location;
            }
            _ => {}
        }
    }
}
