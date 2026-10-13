//! The engine profile's tooling (engine/PROFILE.md): an engine's interface
//! from the author's schemas, and the tables a bundle carries so the device
//! can host the engine without the compiler ([`layout`]).
//!
//! `engine.expr` is `(state, event, timestamp, gps) → {state, view?, wake?}`,
//! evaluated once per event. `event` is exactly one of the host's built-in
//! `gps` (`{fix?, velocity?}`) and `wake` (`{}`), and the author's own
//! events. The built-in events live here (the profile), not in the language.

pub mod layout;

use compiler::diag::Diag;
use serde_json::{Value as Json, json};

/// The most fixes `gps` holds: the tracks region of the C6's data slot
/// (0xA0000 bytes, PROFILE §3) in 32-byte records -- 5.7 h at 1 Hz.
pub const GPS_MAX_ITEMS: u32 = 0xA_0000 / 32;

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
/// author's event can't take a built-in's name. Without author events, only
/// the built-in ones.
pub fn event_schema(author: Option<&Json>) -> Result<Json, Vec<Diag>> {
    let theirs: &[Json] = match author {
        None => &[],
        Some(a) => match a.get("oneOf").and_then(Json::as_array) {
            Some(alts) => alts,
            None => {
                return Err(vec![
                    Diag::interface("E-SCHEMA", "the event schema must be a `oneOf` of variants")
                        .help("{\"oneOf\": [{\"properties\": {\"<name>\": {\"type\": \"object\", …}}, \"required\": [\"<name>\"]}, …]}"),
                ]);
            }
        },
    };
    let mut diags = Vec::new();
    let mut all: Vec<Json> = Vec::new();
    for (name, schema) in builtin_events() {
        if theirs.iter().any(|alt| alt["properties"].get(name).is_some()) {
            diags.push(
                Diag::interface("E-SCHEMA", format!("`{name}` is a built-in event: an author's event can't use the name"))
                    .help("rename the event"),
            );
        }
        all.push(json!({"properties": {name: schema}, "required": [name]}));
    }
    if !diags.is_empty() {
        return Err(diags);
    }
    all.extend(theirs.iter().cloned());
    Ok(json!({"oneOf": all}))
}

/// PROFILE.md §2's `Fix`.
pub fn fix_schema() -> Json {
    let num = json!({"type": "number"});
    json!({"type": "object", "required": ["t", "lat", "lon"],
        "properties": {"t": {"type": "integer", "format": "int64"}, "lat": num, "lon": num, "alt": num}})
}

/// An engine's interface (PROFILE.md §2) from the author's schemas: the
/// state, the view if there is one, and the author's events if any.
pub fn engine_interface(state: &Json, view: Option<&Json>, event: Option<&Json>) -> Result<Json, Vec<Diag>> {
    Ok(json!({
        "inputs": [
            {"name": "state", "schema": state, "nullable": true},
            {"name": "event", "schema": event_schema(event)?},
            {"name": "timestamp", "schema": {"type": "integer", "format": "int64"}},
            {"name": "gps", "schema": {"type": "array", "maxItems": GPS_MAX_ITEMS, "items": fix_schema()}},
        ],
        "output": output_schema(state, view),
        "builtins": declarations(),
    }))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_author_event_cant_take_a_built_in_name() {
        for name in ["gps", "wake"] {
            let ev = json!({"oneOf": [{"properties": {name: {"type": "object"}}, "required": [name]}]});
            let d = event_schema(Some(&ev)).unwrap_err();
            assert_eq!(d.len(), 1);
            assert_eq!(d[0].rule, "E-SCHEMA");
            assert_eq!(d[0].message, format!("`{name}` is a built-in event: an author's event can't use the name"));
        }
    }

    #[test]
    fn an_event_schema_is_a_one_of() {
        let d = event_schema(Some(&json!({"type": "object"}))).unwrap_err();
        assert_eq!(d[0].rule, "E-SCHEMA");
    }

    #[test]
    fn without_author_events_only_the_built_in_ones() {
        let e = event_schema(None).unwrap();
        let names: Vec<&str> = e["oneOf"].as_array().unwrap().iter().map(|a| a["required"][0].as_str().unwrap()).collect();
        assert_eq!(names, ["gps", "wake"]);
    }

    #[test]
    fn the_gps_bound_fills_the_tracks_region() {
        assert_eq!(GPS_MAX_ITEMS, 20_480);
    }
}
