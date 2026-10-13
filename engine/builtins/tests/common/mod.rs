use serde_json::Value as Json;

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
