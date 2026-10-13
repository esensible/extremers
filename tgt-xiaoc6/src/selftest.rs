//! Feature `selftest`: drives the host from inside the firmware, through the
//! same `Host::event` the websocket uses, so a capture of the log shows
//! Choose -> Race -> back working on the device, and what that does to the
//! heap and the main stack, with no client connected. Not for normal
//! builds.
//!
//! Every 30 s: back to Choose; select race; five synthetic GPS fixes (the
//! box may have no GPS); line stbd (the last fix); five more; line port; a
//! 300 s bump stamped now; an event for another package (must be refused);
//! undecodable bytes (refused); finish; back. Each step logs what the host
//! said, the active package and its view (bitwire), and the memory log
//! follows each round.
//!
//! Race's events in bitwire (its event.desc): the variant's index in a byte
//! (`line_stbd` 0, `line_port` 1, `bump_seq` 2, `race_finish` 3), then
//! `bump_seq`'s `seconds` and `tapped_at`, zigzag varints. Those are
//! postcard's bytes for `(u8, i32, i64)` too (checked against wire/js), so
//! postcard writes the bump.

use embassy_time::{Duration, Timer};

use common::{
    host::{Host, Transport},
    nmea::{Fix, GpsUpdate, Velocity},
};
use extreme_protocol::{CHOOSE, HostEvent, PackageId, package_id};

const RACE: PackageId = package_id("race");

const LINE_STBD: &[u8] = &[0];
const LINE_PORT: &[u8] = &[1];
const RACE_FINISH: &[u8] = &[3];

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

/// Five synthetic GPS fixes a second apart, from `step` on: off Sydney
/// Heads, stepping north (~4.4 m a second, 8.6 kn heading 000). The host's
/// clock, as without GPS time.
async fn gps(host: &Host, what: &str, step: &mut u32) {
    for _ in 0..5 {
        let d = *step as f64 * 4.0e-5;
        host.gps_update(GpsUpdate {
            timestamp: None,
            fix: Some(Fix { lat: -33.853 + d, lon: 151.251 }),
            velocity: Some(Velocity { speed: 8.6, heading: 0.0 }),
        })
        .await;
        *step += 1;
        Timer::after(Duration::from_millis(1000)).await;
    }
    let state = host.current_state().await;
    defmt::info!("selftest: {=str} after 5 GPS fixes: view {=[u8]:02x}", what, &state.view[..]);
}

#[embassy_executor::task]
pub async fn selftest_task(host: &'static Host) -> ! {
    Timer::after(Duration::from_secs(8)).await;
    crate::memory::log("selftest: idle before the first round");
    let mut round = 0u32;
    let mut step = 0u32;
    loop {
        round += 1;
        defmt::info!("selftest: round {=u32}", round);
        send(host, "back", HostEvent::SelectDefault, true).await;
        let state = host.current_state().await;
        if state.active != CHOOSE {
            defmt::error!("selftest: FAIL back did not reach Choose");
        }
        send(host, "select race", HostEvent::Select(RACE), true).await;
        gps(host, "race", &mut step).await;
        send(host, "race: line stbd (the last fix)", HostEvent::Engine { package: RACE, bytes: LINE_STBD }, true).await;
        gps(host, "race, stbd set", &mut step).await;
        send(host, "race: line port (the last fix)", HostEvent::Engine { package: RACE, bytes: LINE_PORT }, true).await;
        let mut bump = [0u8; 24];
        let bump = postcard::to_slice(&(2u8, 300i32, host.now() as i64), &mut bump).unwrap();
        send(host, "race: bump 300 s", HostEvent::Engine { package: RACE, bytes: bump }, true).await;
        send(host, "event for another package", HostEvent::Engine { package: 12345, bytes: LINE_STBD }, false).await;
        send(host, "race: undecodable event", HostEvent::Engine { package: RACE, bytes: &[] }, false).await;
        send(host, "select race again (keeps the engine)", HostEvent::Select(RACE), true).await;
        gps(host, "race, in sequence", &mut step).await;
        crate::memory::log("selftest: race active, in sequence, after GPS fixes");
        send(host, "race: finish", HostEvent::Engine { package: RACE, bytes: RACE_FINISH }, true).await;
        send(host, "select 12345 (not installed)", HostEvent::Select(12345), false).await;
        send(host, "back", HostEvent::SelectDefault, true).await;
        crate::memory::log("selftest: after a round, back at Choose");
        Timer::after(Duration::from_secs(30)).await;
    }
}
