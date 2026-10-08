//! The engines the firmware runs, in one place: every target drives
//! [`EngineType`], and `tools/bindgen` generates the clients' JS bindings
//! from [`EngineState`] and [`EngineEvent`]. The order below is the wire
//! order: the chooser is 0, Race 1, TuneSpeed 2 (see
//! [`define_engines!`](extreme_traits::define_engines)); adding an engine at
//! the end keeps the others' tags.

#![no_std]

use extreme_traits::define_engines;

define_engines! {
    pub EngineType {
        Race(extreme_race::Race),
        TuneSpeed(extreme_tune::TuneSpeed<32>),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extreme_traits::{MAX_STATE_SIZE, RawEngine};

    #[test]
    fn wire_tags() {
        assert_eq!(postcard::to_stdvec(&EngineKind::Selector).unwrap(), [0]);
        assert_eq!(postcard::to_stdvec(&EngineKind::Race).unwrap(), [1]);
        assert_eq!(postcard::to_stdvec(&EngineKind::TuneSpeed).unwrap(), [2]);
        assert_eq!(
            postcard::to_stdvec(&EngineEvent::Race(extreme_race::RaceEvent::RaceFinish)).unwrap(),
            [1, 3]
        );
        assert_eq!(&EngineType::default().serialize_state().unwrap()[..], &[0]);
        let tune = EngineType::select(EngineKind::TuneSpeed);
        assert_eq!(tune.serialize_state().unwrap().len(), 13);
        assert!(13 <= MAX_STATE_SIZE);
    }
}
