//! Race on the engine contract (engine/PROFILE.md): `engine.expr` over
//! `state.schema.json`, `view.schema.json` and `event.schema.json`. Clients
//! receive the view.
//!
//! `engine.expr` is `(state, event, timestamp, gps) → {state, view, wake?}`,
//! evaluated once per event. `event` is exactly one of the host's built-in
//! `gps` (`{fix?, velocity?}`) and `wake` (`{}`), and the author's own
//! events. `state` in the output is the new state; `view` is what clients
//! receive, computed from it; `wake` is when the host should next deliver
//! the `wake` event (absent: no timer).
//!
//! The profile's tooling (`engine/profile`) generates the interface from
//! those files, the built-in events, the profile's inputs and output, and the
//! sailing builtins' declarations. `interface.json` is the program's
//! interface, as `langc` is given it; a test checks it is up to date
//! (`UPDATE=1 cargo test` rewrites it). `view.desc` and `event.desc` are the
//! schemas' bitwire descriptors (`wire/js`), checked current the same way.

use serde_json::Value as Json;

pub const PROGRAM: &str = include_str!("engine.expr");
pub const STATE_SCHEMA: &str = include_str!("state.schema.json");
pub const VIEW_SCHEMA: &str = include_str!("view.schema.json");
pub const EVENT_SCHEMA: &str = include_str!("event.schema.json");
pub const VIEW_DESC: &[u8] = include_bytes!("view.desc");
pub const EVENT_DESC: &[u8] = include_bytes!("event.desc");

/// Race's interface, as the profile's tooling generates it.
pub fn interface() -> Json {
    let state: Json = serde_json::from_str(STATE_SCHEMA).unwrap();
    let view: Json = serde_json::from_str(VIEW_SCHEMA).unwrap();
    let event: Json = serde_json::from_str(EVENT_SCHEMA).unwrap();
    engine_profile::engine_interface(&state, Some(&view), Some(&event)).unwrap_or_else(|d| panic!("{d:#?}"))
}
