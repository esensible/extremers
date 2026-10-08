//! The engine runtime: owns the engine, its clock and its timer, and fans
//! state changes out to every connected client.
//!
//! Transports (see [`crate::http`]) only move bytes; everything that touches
//! the engine goes through [`EngineRuntime`].

use core::{cell::RefCell, sync::atomic::Ordering};

use embassy_sync::{
    blocking_mutex::{Mutex as BlockingMutex, raw::CriticalSectionRawMutex},
    channel::Channel,
    mutex::Mutex,
    pubsub::{DynSubscriber, Error as PubSubError, PubSubChannel},
};
use embassy_time::{Duration, Instant, with_timeout};
use portable_atomic::AtomicU64;

use extreme_traits::{MAX_STATE_SIZE, Outcome, RawEngine, StateBytes, Timer};

use crate::{
    ble_settings::{BleCommand, BleStatus, BleStatusView},
    config::{MAX_BLE_CONNECTIONS, MAX_WEB_SOCKETS},
    fmt::Dbg,
    nmea::GpsUpdate,
};

/// A snapshot of the engine state, as sent to clients. Each transport
/// puts the time of sending in front of it (see [`crate::protocol`]).
#[derive(Clone, Debug)]
pub struct StateMessage {
    /// Name of the engine that produced `engine`, for logs.
    pub kind: &'static str,
    /// The engine state, serialized (postcard).
    pub engine: StateBytes,
}

impl StateMessage {
    /// Captures the state with its engine's name. Call with the engine
    /// locked.
    fn capture<E: RawEngine>(engine: &E) -> Result<Self, ()> {
        match engine.serialize_state() {
            Ok(bytes) => Ok(Self {
                kind: engine.kind(),
                engine: bytes,
            }),
            Err(()) => {
                error!(
                    "{} state does not serialize into {} bytes",
                    engine.kind(),
                    MAX_STATE_SIZE
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
    /// State changes for the connected clients, one subscriber per possible
    /// client on every transport. Each message is a complete state, so a
    /// slow client that misses one loses nothing.
    broadcast: PubSubChannel<
        CriticalSectionRawMutex,
        StateMessage,
        1,
        { MAX_WEB_SOCKETS + MAX_BLE_CONNECTIONS },
        1,
    >,
    /// The BLE transport's pairing state and bonds, for the web clients'
    /// settings panel; `None` on a target without BLE (it is then left out
    /// of the state messages and BLE commands are refused).
    ble_status: BlockingMutex<CriticalSectionRawMutex, RefCell<Option<BleStatus>>>,
    /// BLE commands from the web clients to the BLE transport.
    ble_commands: Channel<CriticalSectionRawMutex, BleCommand, 2>,
}

impl<E: RawEngine> EngineRuntime<E> {
    pub fn new(engine: E) -> Self {
        Self {
            engine: Mutex::new(engine),
            tick_offset: AtomicU64::new(0),
            timer_channel: PubSubChannel::new(),
            broadcast: PubSubChannel::new(),
            ble_status: BlockingMutex::new(RefCell::new(None)),
            ble_commands: Channel::new(),
        }
    }

    /// A BLE command from a web client (BLE.md "Pairing a watch"). `Err`
    /// if this target has no BLE transport or the command queue is full.
    pub fn ble_command(&self, command: BleCommand) -> Result<(), ()> {
        if self.ble_status.lock(|s| s.borrow().is_none()) {
            return Err(());
        }
        self.ble_commands.try_send(command).map_err(|_| ())
    }

    /// The next BLE command, for the BLE transport.
    pub async fn next_ble_command(&self) -> BleCommand {
        self.ble_commands.receive().await
    }

    /// Publishes the BLE transport's status (which also marks this target
    /// as having one), and sends every client the current state with it
    /// straight away rather than at the next heartbeat.
    pub async fn set_ble_status(&self, status: BleStatus) {
        self.ble_status.lock(|s| *s.borrow_mut() = Some(status));
        let engine = self.engine.lock().await;
        if let Ok(message) = StateMessage::capture(&*engine) {
            self.broadcast
                .immediate_publisher()
                .publish_immediate(message);
        }
    }

    /// The `ble` field of the websocket state message, as of now (the
    /// pairing window's time left is relative to the moment of sending);
    /// `None` without BLE.
    pub fn ble_status_view(&self) -> Option<BleStatusView> {
        self.ble_status
            .lock(|s| s.borrow().as_ref().map(|s| s.view(Instant::now())))
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

    /// Feeds a client event (the engine event's postcard) to the engine.
    /// `Err` if the engine could not decode it, or it is for an engine that
    /// is not active.
    pub async fn external_event(&self, payload: &[u8]) -> Result<(), ()> {
        let mut engine = self.engine.lock().await;
        let outcome = engine.external_event(self.now(), payload)?;
        self.apply(&engine, outcome);
        Ok(())
    }

    /// The current engine state.
    pub async fn current_state(&self) -> Result<StateMessage, ()> {
        StateMessage::capture(&*self.engine.lock().await)
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
            && let Ok(message) = StateMessage::capture(engine)
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

        fn serialize_state(&self) -> Result<StateBytes, ()> {
            let mut bytes = StateBytes::new();
            bytes.push(self.fired as u8).map_err(|_| ())?;
            Ok(bytes)
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
            assert_eq!(&state.engine[..], &[1]);
            sleep_ms(600).await;
            assert_eq!(runtime.engine.lock().await.fired, 1);
            assert!(updates.try_next_message_pure().is_none());
        };

        block_on(select(runtime.run_timer(), script));
    }

    /// BLE commands are refused until a BLE transport has published its
    /// status; publishing it re-sends the state to the clients.
    #[test]
    fn ble_commands_and_status() {
        let runtime = EngineRuntime::new(TimerEngine { fired: 0 });
        let mut updates = runtime.subscribe().unwrap();

        assert_eq!(runtime.ble_command(BleCommand::Pair), Err(()));
        assert!(runtime.ble_status_view().is_none());

        block_on(runtime.set_ble_status(BleStatus::default()));
        assert_eq!(updates.try_next_message_pure().map(|m| m.kind), Some("TimerEngine"));
        assert_eq!(runtime.ble_status_view().unwrap().pair_ms, 0);

        assert_eq!(runtime.ble_command(BleCommand::Pair), Ok(()));
        assert_eq!(block_on(runtime.next_ble_command()), BleCommand::Pair);
    }
}
