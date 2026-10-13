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
//! This is the profile's tooling for Race: it generates the interface from
//! those files, the built-in events, the profile's inputs and output, and the
//! sailing builtins' declarations. The built-in events live here (the
//! profile), not in the language. `interface.json` is the program's
//! interface, as `langc` is given it; a test checks it is up to date
//! (`UPDATE=1 cargo test` rewrites it).

use serde_json::{Value as Json, json};

pub const PROGRAM: &str = include_str!("engine.expr");
pub const STATE_SCHEMA: &str = include_str!("state.schema.json");
pub const VIEW_SCHEMA: &str = include_str!("view.schema.json");
pub const EVENT_SCHEMA: &str = include_str!("event.schema.json");

/// The host's own events, as variants: a GPS update and the timer's wake.
pub fn builtin_events() -> [(&'static str, Json); 2] {
    let num = json!({"type": "number"});
    let velocity = json!({"type": "object", "required": ["speed", "heading"], "properties": {"speed": num, "heading": num}});
    [
        ("gps", json!({"type": "object", "properties": {"fix": fix_schema(), "velocity": velocity}})),
        ("wake", json!({"type": "object"})),
    ]
}

/// The `event` input: the built-in events' variants, then the author's
/// (`event.schema.json`, a `oneOf` of variants), as one variant group. An
/// author's event can't take a built-in's name.
pub fn event_schema(author: &Json) -> Json {
    let theirs = author["oneOf"].as_array().expect("the event schema is a `oneOf` of variants");
    let mut all: Vec<Json> = Vec::new();
    for (name, schema) in builtin_events() {
        assert!(
            !theirs.iter().any(|alt| alt["properties"].get(name).is_some()),
            "`{name}` is a built-in event: an author's event can't use the name"
        );
        all.push(json!({"properties": {name: schema}, "required": [name]}));
    }
    all.extend(theirs.iter().cloned());
    json!({"oneOf": all})
}

/// PROFILE.md §2's `Fix`.
pub fn fix_schema() -> Json {
    let num = json!({"type": "number"});
    json!({"type": "object", "required": ["t", "lat", "lon"],
        "properties": {"t": {"type": "integer", "format": "int64"}, "lat": num, "lon": num, "alt": num}})
}

/// Race's interface, as the profile's tooling generates it.
pub fn interface() -> Json {
    let state: Json = serde_json::from_str(STATE_SCHEMA).unwrap();
    let view: Json = serde_json::from_str(VIEW_SCHEMA).unwrap();
    let event: Json = serde_json::from_str(EVENT_SCHEMA).unwrap();
    engine_interface(&state, Some(&view), &event)
}

/// An engine's interface (PROFILE.md §2) from the author's schemas: the
/// state, the view if there is one, and the author's events.
pub fn engine_interface(state: &Json, view: Option<&Json>, event: &Json) -> Json {
    json!({
        "inputs": [
            {"name": "state", "schema": state, "nullable": true},
            {"name": "event", "schema": event_schema(event)},
            {"name": "timestamp", "schema": {"type": "integer", "format": "int64"}},
            {"name": "gps", "schema": {"type": "array", "maxItems": 61440, "items": fix_schema()}},
        ],
        "output": output_schema(state, view),
        "builtins": declarations(),
    })
}

/// The output (PROFILE.md §2): `{state, view?, wake?}`. `state` is the new
/// state; `view` is a field, and required, exactly when the author has a
/// view schema; `wake` is optional. No other field is allowed (the checker's
/// `E-FIELD`).
pub fn output_schema(state: &Json, view: Option<&Json>) -> Json {
    let mut properties = serde_json::Map::new();
    properties.insert("state".into(), state.clone());
    let mut required = vec![json!("state")];
    if let Some(v) = view {
        properties.insert("view".into(), v.clone());
        required.push(json!("view"));
    }
    properties.insert(
        "wake".into(),
        json!({"description": "when the host next delivers the `wake` event: epoch ms on the device clock; absent, no timer",
               "type": "integer", "format": "int64"}),
    );
    json!({"type": "object", "required": required, "properties": properties})
}

/// The builtin declarations as the compiler takes them: each
/// `{"$ref": "position"}` replaced by the position schema.
pub fn declarations() -> Json {
    let position: Json = serde_json::from_str(sailing::POSITION).unwrap();
    expand(&serde_json::from_str(sailing::DECLARATIONS).unwrap(), &position)
}

fn expand(v: &Json, position: &Json) -> Json {
    match v {
        Json::Object(o) if o.get("$ref").and_then(Json::as_str) == Some("position") => position.clone(),
        Json::Object(o) => Json::Object(o.iter().map(|(k, e)| (k.clone(), expand(e, position))).collect()),
        Json::Array(a) => Json::Array(a.iter().map(|e| expand(e, position)).collect()),
        _ => v.clone(),
    }
}
