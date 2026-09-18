//! The subset of JSON Schema ArchGen sends to providers, checked locally.
//! A provider that enforces the schema itself is still not trusted: a missing
//! or extra field is a broken answer, never something to default silently.
use serde_json::Value;

/// Supports `type` (string or list), `properties`, `required`,
/// `additionalProperties: false`, `items`, `enum`, `minItems`, `maxItems`,
/// `minLength` and `maxLength`. Errors name the JSON path, e.g. `sections[2].title`.
pub fn check(value: &Value, schema: &Value) -> Result<(), String> {
    check_at(value, schema, "")
}

fn shown(path: &str) -> &str {
    if path.is_empty() {
        "the answer"
    } else {
        path
    }
}

fn type_matches(value: &Value, name: &str) -> bool {
    match name {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn check_at(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    match schema.get("type") {
        Some(Value::String(name)) if !type_matches(value, name) => {
            return Err(format!("{} must be {name}", shown(path)));
        }
        Some(Value::Array(names))
            if !names.iter().filter_map(Value::as_str).any(|name| type_matches(value, name)) =>
        {
            return Err(format!("{} has the wrong type", shown(path)));
        }
        _ => {}
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
        if !allowed.contains(value) {
            return Err(format!("{} is not one of the allowed values", shown(path)));
        }
    }
    if let Some(text) = value.as_str() {
        let length = text.chars().count() as u64;
        if schema.get("minLength").and_then(Value::as_u64).is_some_and(|min| length < min) {
            return Err(format!("{} is too short", shown(path)));
        }
        if schema.get("maxLength").and_then(Value::as_u64).is_some_and(|max| length > max) {
            return Err(format!("{} is too long", shown(path)));
        }
    }
    if let Some(object) = value.as_object() {
        let empty = serde_json::Map::new();
        let properties = schema.get("properties").and_then(Value::as_object).unwrap_or(&empty);
        for name in schema.get("required").and_then(Value::as_array).into_iter().flatten() {
            if let Some(name) = name.as_str() {
                if !object.contains_key(name) {
                    return Err(format!("{} is missing", join(path, name)));
                }
            }
        }
        if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
            if let Some(extra) = object.keys().find(|key| !properties.contains_key(*key)) {
                return Err(format!("{} is not allowed", join(path, extra)));
            }
        }
        for (name, property) in properties {
            if let Some(item) = object.get(name) {
                check_at(item, property, &join(path, name))?;
            }
        }
    }
    if let Some(array) = value.as_array() {
        let count = array.len() as u64;
        if schema.get("minItems").and_then(Value::as_u64).is_some_and(|min| count < min) {
            return Err(format!("{} has too few items", shown(path)));
        }
        if schema.get("maxItems").and_then(Value::as_u64).is_some_and(|max| count > max) {
            return Err(format!("{} has too many items", shown(path)));
        }
        if let Some(items) = schema.get("items") {
            for (index, item) in array.iter().enumerate() {
                check_at(item, items, &format!("{path}[{index}]"))?;
            }
        }
    }
    Ok(())
}

fn join(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.into()
    } else {
        format!("{path}.{name}")
    }
}

/// The first complete JSON object in free text, for providers that cannot be
/// held to a schema and wrap their answer in prose or a code fence.
pub fn first_json_object(text: &str) -> Result<Value, String> {
    let mut search = 0;
    while let Some(offset) = text[search..].find('{') {
        let start = search + offset;
        if let Some(end) = object_end(&text[start..]) {
            if let Ok(value) = serde_json::from_str::<Value>(&text[start..start + end]) {
                return Ok(value);
            }
        }
        search = start + 1;
    }
    Err("the answer contains no JSON object".into())
}

/// Byte length of the balanced `{…}` at the start of `text`, honouring strings.
fn object_end(text: &str) -> Option<usize> {
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    for (index, c) in text.char_indices() {
        if quoted {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => quoted = false,
                _ => {}
            }
        } else {
            match c {
                '"' => quoted = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(index + 1);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// For a provider without schema enforcement: put the schema in the prompt,
/// and give the model one chance to correct an answer that violates it.
/// `answer` receives the full prompt text and returns the provider's raw text.
pub fn by_prompt(
    system: &str,
    user: &str,
    schema: &Value,
    mut answer: impl FnMut(&str) -> Result<String, String>,
) -> Result<Value, String> {
    let mut prompt = format!(
        "{system}\n\nAnswer with exactly one JSON object that matches this JSON Schema, and nothing else (no prose, no code fence):\n{schema}\n\n{user}"
    );
    let mut last = String::new();
    for attempt in 0..2 {
        let text = answer(&prompt)?;
        match first_json_object(&text).and_then(|value| check(&value, schema).map(|_| value)) {
            Ok(value) => return Ok(value),
            Err(violation) => {
                if attempt == 0 {
                    prompt.push_str(&format!(
                        "\n\nYour previous answer was rejected: {violation}. Return the corrected JSON object only."
                    ));
                }
                last = violation;
            }
        }
    }
    Err(format!("The assistant did not return valid JSON after one correction: {last}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({"type":"object","additionalProperties":false,"required":["title","items"],"properties":{
            "title":{"type":"string","minLength":1},
            "kind":{"enum":["a","b"]},
            "items":{"type":"array","maxItems":2,"items":{"type":"object","additionalProperties":false,"required":["id"],"properties":{"id":{"type":"string"},"n":{"type":["integer","null"]}}}}}})
    }

    #[test]
    fn accepts_a_matching_value_and_names_the_path_of_a_violation() {
        assert!(check(&json!({"title":"T","kind":"a","items":[{"id":"x","n":null},{"id":"y","n":3}]}), &schema()).is_ok());
        for (value, expected) in [
            (json!([]), "the answer must be object"),
            (json!({"items":[]}), "title is missing"),
            (json!({"title":"","items":[]}), "title is too short"),
            (json!({"title":"T","items":[],"extra":1}), "extra is not allowed"),
            (json!({"title":"T","kind":"c","items":[]}), "kind is not one of the allowed values"),
            (json!({"title":"T","items":[{"id":1}]}), "items[0].id must be string"),
            (json!({"title":"T","items":[{"id":"x","n":1.5}]}), "items[0].n has the wrong type"),
            (json!({"title":"T","items":[{"id":"x","other":true}]}), "items[0].other is not allowed"),
            (json!({"title":"T","items":[{"id":"1"},{"id":"2"},{"id":"3"}]}), "items has too many items"),
        ] {
            assert_eq!(check(&value, &schema()).unwrap_err(), expected);
        }
    }

    #[test]
    fn finds_the_object_inside_prose_fences_and_tricky_strings() {
        assert_eq!(first_json_object("Here you go:\n```json\n{\"a\": {\"b\": \"}\\\"{\"}}\n```\nDone.").unwrap(), json!({"a":{"b":"}\"{"}}));
        assert_eq!(first_json_object("{not json} then {\"ok\":true}").unwrap(), json!({"ok":true}));
        assert!(first_json_object("no object here").is_err());
        assert!(first_json_object("{\"unfinished\": ").is_err());
    }

    #[test]
    fn by_prompt_retries_once_with_the_violation() {
        let schema = json!({"type":"object","additionalProperties":false,"required":["ok"],"properties":{"ok":{"type":"boolean"}}});
        let mut prompts = Vec::new();
        let value = by_prompt("SYSTEM", "USER", &schema, |prompt| {
            prompts.push(prompt.to_string());
            Ok(if prompts.len() == 1 { "{\"ok\":\"yes\"}".into() } else { "Sure: {\"ok\":true}".into() })
        })
        .unwrap();
        assert_eq!(value, json!({"ok":true}));
        assert!(prompts[0].starts_with("SYSTEM") && prompts[0].ends_with("USER") && prompts[0].contains("\"required\""));
        assert!(prompts[1].contains("ok must be boolean"));

        let mut calls = 0;
        let error = by_prompt("", "", &schema, |_| {
            calls += 1;
            Ok("nothing".into())
        })
        .unwrap_err();
        assert_eq!(calls, 2);
        assert!(error.contains("no JSON object"), "{error}");
        assert_eq!(by_prompt("", "", &schema, |_| Err("offline".into())).unwrap_err(), "offline");
    }
}
