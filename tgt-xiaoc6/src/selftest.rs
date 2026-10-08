//! Feature `selftest`: drives the host from inside the firmware, through the
//! same `Host::event` the websocket uses, so a capture of the log shows
//! Choose -> Race -> Tune -> back working on the device, and what that does
//! to the heap and the main stack, with no client connected. Not for normal
//! builds.
//!
//! Every 30 s: back to Choose; select race; line stbd; a 300 s bump stamped
//! now; an event for tune while race is active (must be refused); finish;
//! select tune; race again (a fresh engine); back. Each step logs what the
//! host said and the active package, and the memory log follows each round.

use embassy_time::{Duration, Timer};

use common::{
    host::{Host, Transport},
    nmea::GpsUpdate,
};
use extreme_sdk::{Fix, Velocity};
use extreme_protocol::{CHOOSE, HostEvent, PackageId, package_id};

const RACE: PackageId = package_id("race");
const TUNE: PackageId = package_id("tune");

async fn send(host: &Host, what: &str, event: HostEvent<'_>, want_ok: bool) {
    let mut buf = [0u8; 64];
    let Ok(bytes) = postcard::to_slice(&event, &mut buf) else {
        defmt::error!("selftest: {=str}: does not encode", what);
        return;
    };
    let result = host.event(bytes, Transport::WebSocket).await;
    let state = host.current_state().await;
    let pass = result.is_ok() == want_ok;
    if pass {
        defmt::info!(
            "selftest: {=str}: {=str}; active {=u32} ({=str}), view {=[u8]:02x}",
            what,
            if result.is_ok() { "accepted" } else { "refused" },
            state.active,
            state.name,
            &state.view[..]
        );
    } else {
        defmt::error!(
            "selftest: FAIL {=str}: {:?}; active {=u32} ({=str})",
            what,
            defmt::Debug2Format(&result),
            state.active,
            state.name
        );
    }
}

/// Five synthetic GPS updates a second apart (the box may have no fix
/// indoors), so the engine's on_location is compiled and run too: off
/// Sydney Heads heading 045 at 5-7 kn. The host's clock, as without GPS
/// time.
async fn gps(host: &Host, what: &str) {
    for i in 0..5u32 {
        let d = i as f64 * 1.8e-5;
        host.gps_update(GpsUpdate {
            timestamp: None,
            fix: Some(Fix { lat: -33.85 + d, lon: 151.25 + d }),
            velocity: Some(Velocity { speed: 5.0 + i as f64 * 0.5, heading: 45.0 }),
        })
        .await;
        Timer::after(Duration::from_millis(1000)).await;
    }
    let state = host.current_state().await;
    defmt::info!("selftest: {=str} after 5 GPS updates: view {=[u8]:02x}", what, &state.view[..]);
}

fn race_event(event: &[u8], out: &mut [u8; 32]) -> usize {
    out[..event.len()].copy_from_slice(event);
    event.len()
}

#[embassy_executor::task]
pub async fn selftest_task(host: &'static Host) -> ! {
    Timer::after(Duration::from_secs(8)).await;
    crate::memory::log("selftest: idle before the first round");
    let mut round = 0u32;
    loop {
        round += 1;
        defmt::info!("selftest: round {=u32}", round);
        send(host, "back", HostEvent::SelectDefault, true).await;
        let state = host.current_state().await;
        if state.active != CHOOSE {
            defmt::error!("selftest: FAIL back did not reach Choose");
        }
        send(host, "select race", HostEvent::Select(RACE), true).await;
        Timer::after(Duration::from_millis(500)).await;
        // RaceEvent::LineStbd = [0]
        send(host, "race: line stbd", HostEvent::Engine { package: RACE, bytes: &[0] }, true).await;
        // RaceEvent::BumpSeq { seconds: 300, timestamp: now }
        let mut ev = [0u8; 32];
        let n = {
            let mut tmp = [0u8; 16];
            let bump = postcard::to_slice(&(2u8, 300i32, host.now()), &mut tmp).unwrap();
            race_event(bump, &mut ev)
        };
        send(host, "race: bump 300 s", HostEvent::Engine { package: RACE, bytes: &ev[..n] }, true).await;
        send(host, "tune event while race is active", HostEvent::Engine { package: TUNE, bytes: &[] }, false).await;
        send(host, "race: undecodable event", HostEvent::Engine { package: RACE, bytes: &[9] }, false).await;
        send(host, "select race again (keeps the engine)", HostEvent::Select(RACE), true).await;
        Timer::after(Duration::from_millis(500)).await;
        gps(host, "race").await;
        crate::memory::log("selftest: race active, in sequence, after GPS updates");
        // RaceEvent::RaceFinish = [3]
        send(host, "race: finish", HostEvent::Engine { package: RACE, bytes: &[3] }, true).await;
        send(host, "select tune", HostEvent::Select(TUNE), true).await;
        gps(host, "tune").await;
        crate::memory::log("selftest: tune active, after GPS updates");
        send(host, "race event while tune is active", HostEvent::Engine { package: RACE, bytes: &[0] }, false).await;
        send(host, "select race (fresh)", HostEvent::Select(RACE), true).await;
        send(host, "select 12345 (not installed)", HostEvent::Select(12345), false).await;
        send(host, "back", HostEvent::SelectDefault, true).await;
        crate::memory::log("selftest: after a round, back at Choose");
        Timer::after(Duration::from_secs(30)).await;
    }
}
