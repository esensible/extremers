//! The wasm host's `gps` track: the newest fixes in a `VecDeque`, up to a
//! capacity given when the host is made ([`DEVICE_FIXES`] by default, what
//! the C6's flash ring holds), so an engine's look-backs see what they see
//! on the device.
//!
//! The C6's [`FlashTrack`](common::track::FlashTrack) is a ring of 4 KB
//! sectors: once full it drops its oldest 128 fixes at a time (a sector
//! erased), so its `gps` holds between 20,352 and 20,480 fixes; this one
//! drops the oldest fix at a time, holding exactly the capacity.
//!
//! Cost: 24 bytes per fix held (`t`, `lat`, `lon`), allocated as fixes
//! arrive: 480 KB of the module's memory at 20,480 fixes, nothing before the
//! first. The host appends through one handle ([`SharedTrack`], lent to
//! `Host::with_track`); the studio reads the count through another.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard},
};

use common::track::{Track, TrackFix, TrackLog};

/// The fixes the C6 holds: its track ring's records (`GPS_MAX_ITEMS`, the
/// `maxItems` of every engine's `gps`).
pub const DEVICE_FIXES: u32 = engine_profile::GPS_MAX_ITEMS;

/// The newest `capacity` fixes, oldest first.
#[derive(Debug)]
pub struct Fixes {
    fixes: VecDeque<(i64, f64, f64)>,
    capacity: usize,
}

impl Fixes {
    pub fn new(capacity: u32) -> Fixes {
        Fixes { fixes: VecDeque::new(), capacity: capacity as usize }
    }

    pub fn len(&self) -> u32 {
        self.fixes.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.fixes.is_empty()
    }

    pub fn capacity(&self) -> u32 {
        self.capacity as u32
    }

    pub fn get(&self, i: u32) -> Option<TrackFix> {
        self.fixes.get(i as usize).map(|&(t, lat, lon)| TrackFix { t, lat, lon, alt: None })
    }

    pub fn last(&self) -> Option<TrackFix> {
        self.fixes.len().checked_sub(1).and_then(|i| self.get(i as u32))
    }

    /// As `common::track`'s tracks append: `t` is `now`, or one more than
    /// the last fix's if the clock has not moved past it.
    pub fn append(&mut self, now: i64, lat: f64, lon: f64) -> i64 {
        let t = match self.fixes.back() {
            Some(&(last, ..)) => now.max(last.saturating_add(1)),
            None => now,
        };
        if self.capacity > 0 {
            if self.fixes.len() == self.capacity {
                self.fixes.pop_front();
            }
            self.fixes.push_back((t, lat, lon));
        }
        t
    }
}

/// [`Fixes`] shared between the host (which appends) and the handle (which
/// reads): a `Mutex` because the host wants a `Send` track; the module has
/// one thread, so it is never contended.
#[derive(Clone, Debug)]
pub struct SharedTrack(Arc<Mutex<Fixes>>);

impl SharedTrack {
    pub fn new(capacity: u32) -> SharedTrack {
        SharedTrack(Arc::new(Mutex::new(Fixes::new(capacity))))
    }

    pub fn lock(&self) -> MutexGuard<'_, Fixes> {
        // a panic while it was held aborts the module: never poisoned
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Track for SharedTrack {
    fn len(&self) -> u32 {
        self.lock().len()
    }

    fn get(&self, i: u32) -> Option<TrackFix> {
        self.lock().get(i)
    }
}

impl TrackLog for SharedTrack {
    fn append(&mut self, now: i64, lat: f64, lon: f64) -> i64 {
        self.lock().append(now, lat, lon)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_newest_with_t_strictly_increasing() {
        let mut f = Fixes::new(3);
        assert_eq!(f.append(10, 1.0, 1.0), 10);
        assert_eq!(f.append(10, 2.0, 2.0), 11, "the clock has not moved: one more than the last");
        assert_eq!(f.append(5, 3.0, 3.0), 12);
        assert_eq!(f.append(20, 4.0, 4.0), 20);
        assert_eq!(f.len(), 3);
        assert_eq!(f.get(0).map(|x| (x.t, x.lat)), Some((11, 2.0)), "the oldest went");
        assert_eq!(f.last().map(|x| x.t), Some(20));
        assert_eq!(f.get(3), None);
        let mut none = Fixes::new(0);
        assert_eq!(none.append(7, 0.0, 0.0), 7);
        assert!(none.is_empty());
    }
}
