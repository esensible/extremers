//! The engine runtime: owns the engine, its clock and its timer, and fans
//! state changes out to every connected client.
//!
//! Transports (see [`crate::http`]) only move bytes; everything that touches
//! the engine goes through [`EngineRuntime`].

use core::sync::atomic::Ordering;

use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    mutex::Mutex,
    pubsub::{DynSubscriber, Error as PubSubError, PubSubChannel},
};
use embassy_time::{Duration, Instant, with_timeout};
use portable_atomic::AtomicU64;

use extreme_traits::{CompactState, MAX_MESSAGE_SIZE, Outcome, RawEngine, StateJson, Timer};

use crate::{config::MAX_WEB_SOCKETS, fmt::Dbg, nmea::GpsUpdate};

/// A snapshot of the engine state, as sent to clients, in both encodings.
#[derive(Clone, Debug)]
pub struct StateMessage {
    /// Name of the engine that produced `json`.
    pub kind: &'static str,
    /// The engine state, serialized (the websocket clients).
    pub json: StateJson,
    /// The same state in the compact binary form (BLE.md), with its times
    /// relative to the moment it was captured.
    pub compact: CompactState,
}

impl StateMessage {
    /// Captures name and both encodings of the state together, so they
    /// always describe the same engine at the same moment. Call with the
    /// engine locked.
    fn capture<E: RawEngine>(engine: &E, now: u64) -> Result<Self, ()> {
        match engine.serialize_state() {
            Ok(json) => Ok(Self {
                kind: engine.kind(),
                json,
                compact: engine.compact_state(now),
            }),
            Err(()) => {
                error!(
                    "{} state does not serialize into {} bytes",
                    engine.kind(),
                    MAX_MESSAGE_SIZE
                );
                Err(())
            }
        }
    }
}

pub struct EngineRuntime<E: RawEngine> {
    engine: Mutex<CriticalSectionRawMutex, E>,
    /// Added to uptime to get epoch milliseconds; 0 until the first GPS time.
    tick_offset: AtomicU64,
    /// Timer instructions for [`run_timer`](Self::run_timer). Only ever
    /// published with the engine locked, so they arrive in the order the
    /// engine issued them; only the latest one matters, hence capacity 1.
    timer_channel: PubSubChannel<CriticalSectionRawMutex, Timer, 1, 1, 1>,
    /// State changes for the connected clients. Each message is a complete
    /// state, so a slow client that misses one loses nothing.
    broadcast: PubSubChannel<CriticalSectionRawMutex, StateMessage, 1, MAX_WEB_SOCKETS, 1>,
}

impl<E: RawEngine> EngineRuntime<E> {
    pub fn new(engine: E) -> Self {
        Self {
            engine: Mutex::new(engine),
            tick_offset: AtomicU64::new(0),
            timer_channel: PubSubChannel::new(),
            broadcast: PubSubChannel::new(),
        }
    }

    /// Current time in epoch milliseconds, the units of GPS timestamps.
    /// Until the first GPS time arrives this is just uptime.
    ///
    /// Known limitation: the clock jumps forward when the first GPS time
    /// arrives, so a timer set before then (its deadline is in uptime
    /// units) fires immediately afterwards. In practice a start sequence is
    /// never begun before the GPS has a fix.
    pub fn now(&self) -> u64 {
        Instant::now().as_millis() + self.tick_offset.load(Ordering::Relaxed)
    }

    /// Feeds a GPS update to the engine. The first GPS time also sets the
    /// clock used by [`now`](Self::now).
    pub async fn gps_update(&self, update: GpsUpdate) {
        let timestamp = match update.timestamp {
            Some(timestamp) => {
                if self.tick_offset.load(Ordering::Relaxed) == 0 {
                    let uptime = Instant::now().as_millis();
                    self.tick_offset
                        .store(timestamp.saturating_sub(uptime), Ordering::Relaxed);
                    info!("clock set from GPS: {} ms since epoch", timestamp);
                }
                timestamp
            }
            None => self.now(),
        };

        let mut engine = self.engine.lock().await;
        let outcome = engine.location_event(timestamp, update.fix, update.velocity);
        self.apply(&engine, outcome);
    }

    /// Feeds a client event (JSON) to the engine. `Err` if the engine could
    /// not decode it.
    pub async fn external_event(&self, payload: &[u8]) -> Result<(), ()> {
        let mut engine = self.engine.lock().await;
        let outcome = engine.external_event(self.now(), payload)?;
        self.apply(&engine, outcome);
        Ok(())
    }

    /// Feeds a client event in the compact binary form (BLE.md) to the
    /// engine. `Err` if it was not understood.
    pub async fn compact_event(&self, payload: &[u8]) -> Result<(), ()> {
        let mut engine = self.engine.lock().await;
        let outcome = engine.compact_event(self.now(), payload)?;
        self.apply(&engine, outcome);
        Ok(())
    }

    /// The current engine state.
    pub async fn current_state(&self) -> Result<StateMessage, ()> {
        StateMessage::capture(&*self.engine.lock().await, self.now())
    }

    /// Subscribes to state changes. Subscribe before reading
    /// [`current_state`](Self::current_state) so no change is missed.
    pub fn subscribe(&self) -> Result<DynSubscriber<'_, StateMessage>, PubSubError> {
        self.broadcast.dyn_subscriber()
    }

    /// Looks up a static file of the active engine; `path` has no leading
    /// slash. The engine is only locked for the lookup.
    pub async fn static_file(&self, path: &str) -> Option<&'static [u8]> {
        self.engine.lock().await.get_static(path)
    }

    /// Delivers the engine's timer events. Run exactly once, in its own task.
    pub async fn run_timer(&self) -> ! {
        let mut instructions = loop {
            match self.timer_channel.dyn_subscriber() {
                Ok(subscriber) => break subscriber,
                Err(e) => {
                    error!("timer: cannot subscribe ({:?}), retrying", Dbg(&e));
                    embassy_time::Timer::after(Duration::from_secs(10)).await;
                }
            }
        };

        // when the engine wants its next timer event, in epoch milliseconds
        let mut pending: Option<u64> = None;

        loop {
            let Some(at) = pending else {
                pending = next_pending(pending, instructions.next_message_pure().await);
                continue;
            };

            let delay = at.saturating_sub(self.now());
            debug!("timer: due in {} ms", delay);
            match with_timeout(
                Duration::from_millis(delay),
                instructions.next_message_pure(),
            )
            .await
            {
                Ok(instruction) => pending = next_pending(pending, instruction),
                Err(_) => {
                    let mut engine = self.engine.lock().await;
                    // instructions are published under the engine lock, so
                    // anything that arrived while we waited for it is newer
                    // than this expiry and replaces it
                    if let Some(instruction) = instructions.try_next_message_pure() {
                        pending = next_pending(pending, instruction);
                        continue;
                    }
                    debug!("timer: firing for {}", at);
                    pending = None;
                    let outcome = engine.timer_event(at);
                    self.apply(&engine, outcome);
                }
            }
        }
    }

    /// Acts on the result of an engine event: broadcasts the new state if it
    /// changed and passes any timer instruction to [`run_timer`]. Call with
    /// the engine locked, so that the state and its `kind` are captured
    /// together and timer instructions stay in order.
    ///
    /// [`run_timer`]: Self::run_timer
    fn apply(&self, engine: &E, outcome: Outcome) {
        if outcome.changed
            && let Ok(message) = StateMessage::capture(engine, self.now())
        {
            self.broadcast
                .immediate_publisher()
                .publish_immediate(message);
        }

        if outcome.timer != Timer::Keep {
            debug!("timer: {:?}", Dbg(&outcome.timer));
            self.timer_channel
                .immediate_publisher()
                .publish_immediate(outcome.timer);
        }
    }
}

/// The pending timer after `instruction`.
fn next_pending(pending: Option<u64>, instruction: Timer) -> Option<u64> {
    match instruction {
        Timer::Keep => pending,
        Timer::At(at) => Some(at),
        Timer::Cancel => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embassy_futures::{block_on, select::select};
    use extreme_traits::{Fix, Velocity};

    /// Counts timer events; client events `at:<delay>` and `cancel` set and
    /// cancel its timer.
    struct TimerEngine {
        fired: u32,
    }

    impl RawEngine for TimerEngine {
        fn kind(&self) -> &'static str {
            "TimerEngine"
        }

        fn serialize_state(&self) -> Result<StateJson, ()> {
            let mut json = StateJson::new();
            json.push(b'0' + self.fired as u8).map_err(|_| ())?;
            Ok(json)
        }

        fn external_event(&mut self, timestamp: u64, event: &[u8]) -> Result<Outcome, ()> {
            let timer = match event {
                b"cancel" => Timer::Cancel,
                _ => {
                    let delay = event.strip_prefix(b"at:").ok_or(())?;
                    let delay: u64 = core::str::from_utf8(delay)
                        .map_err(|_| ())?
                        .parse()
                        .map_err(|_| ())?;
                    Timer::At(timestamp + delay)
                }
            };
            Ok(Outcome {
                changed: false,
                timer,
            })
        }

        fn location_event(&mut self, _: u64, _: Option<Fix>, _: Option<Velocity>) -> Outcome {
            Outcome::NONE
        }

        fn timer_event(&mut self, _: u64) -> Outcome {
            self.fired += 1;
            Outcome::CHANGED
        }

        fn get_static(&self, _: &str) -> Option<&'static [u8]> {
            None
        }

        fn kind_code(&self) -> u8 {
            1
        }

        fn compact_state(&self, _now: u64) -> CompactState {
            CompactState::from_slice(&[1, self.fired as u8]).unwrap()
        }

        fn compact_event(&mut self, timestamp: u64, event: &[u8]) -> Result<Outcome, ()> {
            match *event {
                [0x20] => RawEngine::external_event(self, timestamp, b"at:30"),
                _ => Err(()),
            }
        }
    }

    async fn sleep_ms(ms: u64) {
        embassy_time::Timer::after(Duration::from_millis(ms)).await
    }

    #[test]
    fn timer_fires_and_cancels() {
        let runtime = EngineRuntime::new(TimerEngine { fired: 0 });
        let mut updates = runtime.subscribe().unwrap();

        let script = async {
            assert!(runtime.external_event(b"nonsense").await.is_err());

            // a cancelled timer never fires
            runtime.external_event(b"at:30").await.unwrap();
            runtime.external_event(b"cancel").await.unwrap();
            sleep_ms(100).await;
            assert_eq!(runtime.engine.lock().await.fired, 0);

            // a replaced timer fires once, at the new time
            runtime.external_event(b"at:500").await.unwrap();
            runtime.external_event(b"at:20").await.unwrap();
            let state = updates.next_message_pure().await;
            assert_eq!(state.kind, "TimerEngine");
            assert_eq!(&state.json[..], b"1");
            assert_eq!(&state.compact[..], &[1, 1]);
            assert!(runtime.compact_event(&[0x99]).await.is_err());
            sleep_ms(600).await;
            assert_eq!(runtime.engine.lock().await.fired, 1);
            assert!(updates.try_next_message_pure().is_none());
        };

        block_on(select(runtime.run_timer(), script));
    }
}
