//! The engine's `gps` list: the fixes since boot, oldest first
//! (engine/PROFILE.md §3).
//!
//! A [`Track`] holds the fixes; [`GpsList`] serves one to the VM as the
//! `gps` input (`vm::ListInput`), each fix's fields at the leaves the
//! engine's layout names. For now the track is a ring in RAM
//! ([`RamTrack`]): the newest `N` fixes, gone at a reboot. A fix is `{t,
//! lat, lon, alt?}`; the RAM track keeps no altitude, so `alt` is always
//! absent. `t` is the device clock, strictly increasing.

use bundle::Layout;
use vm::{ListInput, Value};

/// One fix in a track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackFix {
    /// Device clock, epoch ms; strictly increasing along the track.
    pub t: i64,
    pub lat: f64,
    pub lon: f64,
}

/// Fixes, oldest first.
pub trait Track {
    fn len(&self) -> u32;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Fix `i` (`None` past the end).
    fn get(&self, i: u32) -> Option<TrackFix>;
}

/// A fix's leaves within an item of `gps`, from the engine's layout
/// (`fix_*`, item-relative).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixLeaves {
    pub t: u16,
    pub lat: u16,
    pub lon: u16,
    pub alt: u16,
}

impl FixLeaves {
    pub fn of(layout: &Layout) -> FixLeaves {
        FixLeaves {
            t: layout.fix_t,
            lat: layout.fix_lat,
            lon: layout.fix_lon,
            alt: layout.fix_alt,
        }
    }
}

/// `track` as an engine's `gps` input.
pub struct GpsList<'t> {
    pub track: &'t dyn Track,
    pub leaves: FixLeaves,
}

impl<'a> ListInput<'a> for GpsList<'_> {
    fn len(&self) -> u32 {
        self.track.len()
    }

    fn read(&self, item: u32, leaf: u16) -> Value<'a> {
        let Some(f) = self.track.get(item) else {
            return Value::Nil;
        };
        let l = self.leaves;
        if leaf == l.t {
            Value::Int(f.t)
        } else if leaf == l.lat {
            Value::Float(f.lat)
        } else if leaf == l.lon {
            Value::Float(f.lon)
        } else {
            // `alt` (`l.alt`): not kept
            Value::Nil
        }
    }
}

/// The newest `N` fixes, in RAM.
pub struct RamTrack<const N: usize> {
    fixes: [TrackFix; N],
    /// Where the next fix goes.
    head: usize,
    /// Fixes held, at most `N`.
    len: usize,
}

impl<const N: usize> RamTrack<N> {
    pub const fn new() -> Self {
        RamTrack {
            fixes: [TrackFix { t: 0, lat: 0.0, lon: 0.0 }; N],
            head: 0,
            len: 0,
        }
    }

    /// Appends a fix taken at `now`; its `t`: `now`, or one more than the
    /// last fix's if the clock has not moved past it.
    pub fn append(&mut self, now: i64, lat: f64, lon: f64) -> i64 {
        let t = match self.last() {
            Some(last) => now.max(last.t.saturating_add(1)),
            None => now,
        };
        if N > 0 {
            self.fixes[self.head] = TrackFix { t, lat, lon };
            self.head = (self.head + 1) % N;
            self.len = (self.len + 1).min(N);
        }
        t
    }

    /// The newest fix.
    pub fn last(&self) -> Option<TrackFix> {
        (self.len > 0).then(|| self.fixes[(self.head + N - 1) % N])
    }
}

impl<const N: usize> Default for RamTrack<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Track for RamTrack<N> {
    fn len(&self) -> u32 {
        self.len as u32
    }

    fn get(&self, i: u32) -> Option<TrackFix> {
        let i = i as usize;
        (i < self.len).then(|| self.fixes[(self.head + N - self.len + i) % N])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ring_of_the_newest_fixes() {
        let mut track = RamTrack::<3>::new();
        assert!(track.len() == 0 && track.last().is_none());
        assert_eq!(track.append(100, 1.0, 2.0), 100);
        // the clock did not move: one ms after the last
        assert_eq!(track.append(100, 3.0, 4.0), 101);
        assert_eq!(track.append(50, 5.0, 6.0), 102);
        assert_eq!(track.append(200, 7.0, 8.0), 200);
        assert_eq!(track.len(), 3);
        let ts: [i64; 3] = core::array::from_fn(|i| track.get(i as u32).unwrap().t);
        assert_eq!(ts, [101, 102, 200]);
        assert_eq!(track.last(), Some(TrackFix { t: 200, lat: 7.0, lon: 8.0 }));
        assert_eq!(track.get(3), None);
    }

    #[test]
    fn served_at_the_layouts_leaves() {
        let mut track = RamTrack::<4>::new();
        track.append(10, -33.0, 151.0);
        // Race's: alt 0, lat 1, lon 2, t 3
        let gps = GpsList { track: &track, leaves: FixLeaves { t: 3, lat: 1, lon: 2, alt: 0 } };
        let l: &dyn ListInput = &gps;
        assert_eq!(l.len(), 1);
        assert_eq!(
            [l.read(0, 0), l.read(0, 1), l.read(0, 2), l.read(0, 3)],
            [Value::Nil, Value::Float(-33.0), Value::Float(151.0), Value::Int(10)]
        );
        assert_eq!(l.read(1, 3), Value::Nil);
    }
}
