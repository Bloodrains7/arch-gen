//! HTTP providers through the system curl: Ollama, OpenAI, Gemini, Anthropic.
//! See docs/AI-REWORK-DESIGN.md.
//!
//! Every request is a curl subprocess driven by `process::run`: the URL, headers (with the
//! key) and the JSON body all live in one `--config -` text read from stdin, so nothing
//! sensitive is ever a command-line argument, and the body is never written to disk (`@file`).
//! Builders and parsers below are pure and unit-tested on their own; only `complete` and
//! `ollama_models` touch a process.
use super::{process, process::Cancel, Completion, ProviderKind, Route};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";
const GEMINI_URL_TEMPLATE: &str = "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent";
const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const OLLAMA_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);

/// OpenAI, Gemini and Anthropic count hidden reasoning against the same cap as the answer
/// (`max_completion_tokens`, `maxOutputTokens`, `max_tokens`) and think by default on current
/// models, so a budget sized only for the answer can be exhausted before any JSON is written.
/// Adding a fixed allowance on top keeps `Completion::max_output_tokens` meaning "how long the
/// answer may be" for callers, while still giving the provider room to think. Ollama needs no
/// such allowance: `num_predict` is applied per completion pass, not to thinking and the answer
/// combined (see `complete_ollama`).
const CLOUD_REASONING_ALLOWANCE: u32 = 8_000;

/// A value inside curl's `key = "value"` config syntax: backslashes and quotes are the only
/// characters that syntax treats specially inside a quoted value. curl reads its config one
/// line at a time, so a raw control character — a line feed above all — would end the line
/// early and let the rest of the value be read back as a second, attacker-chosen option (a
/// forged `proxy =` or a second `url =`); no legitimate value here ever needs one, so `quote`
/// refuses rather than trying to escape it.
fn quote(value: &str) -> Result<String, String> {
    if value.chars().any(|c| c.is_control()) {
        return Err("A value for curl's config contained a control character.".into());
    }
    Ok(format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")))
}

/// The Windows-inbox curl first (present since Windows 10 1803, never shadowed by a stray PATH
/// entry), then whatever `curl` resolves to on PATH.
fn locate_curl() -> Option<PathBuf> {
    let system_root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let inbox = Path::new(&system_root).join("System32").join("curl.exe");
    if inbox.is_file() {
        Some(inbox)
    } else {
        process::locate("curl")
    }
}

/// Runs curl with `config` on its stdin from a private, empty working directory. `Ok` even on
/// a non-2xx HTTP status — curl exits 0 on a 4xx/5xx body; the caller's parser decides.
/// `outer_timeout` is the hard process-kill deadline, 5 s past curl's own `max-time` (already
/// baked into `config`) so curl gets a chance to report its own timeout before ArchGen force-kills it.
fn run_curl(curl: &Path, config: &str, outer_timeout: Duration, cancel: &Cancel) -> Result<process::Output, String> {
    let dir = process::TempDir::new()?;
    let mut cmd = Command::new(curl);
    cmd.args(["-q", "-sS", "--config", "-"]).current_dir(dir.path());
    process::run(cmd, config, &process::Limits::new(outer_timeout), cancel)
}

/// `Limits.timeout` = `route.timeout` + 5 s (the contract's grace period), kept out of curl's
/// own `max-time` so the outer kill is strictly later than curl's own deadline.
fn outer_timeout(max_time: Duration) -> Duration {
    max_time + Duration::from_secs(5)
}

/// The shared read order every provider follows: (1) a JSON body, even on an HTTP error status,
/// goes to the caller's parser; (2) otherwise a failed curl reports its own (truncated) stderr;
/// (3) a "successful" curl with a non-JSON body is the last resort.
fn http_json_answer(output: Result<process::Output, String>, parse: impl FnOnce(&Value) -> Result<Value, String>) -> Result<Value, String> {
    let output = output?;
    if let Ok(value) = serde_json::from_str::<Value>(&output.stdout) {
        return parse(&value);
    }
    if !output.success {
        return Err(process::truncate(&output.stderr, 300));
    }
    Err("The reply was not JSON.".into())
}

// ── Ollama ───────────────────────────────────────────────────────────────────

/// `disable_thinking` sends `"think": false`; used only for the trivial `TEST_MAX_OUTPUT_TOKENS`
/// probe, where a thinking-capable model could spend the whole tiny budget reasoning and never
/// write the one-word answer. A real rework keeps Ollama's own default (thinking on for a
/// capable model) because `num_predict` bounds each completion pass separately, so its much
/// larger budget is not actually starved by thinking the way the probe's is.
fn ollama_body(model: &str, system: &str, user: &str, schema: &Value, max_output_tokens: u32, disable_thinking: bool) -> Value {
    let mut body = json!({
        "model": model,
        "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
        "format": schema,
        "stream": false,
        "options": {"num_predict": max_output_tokens},
    });
    if disable_thinking {
        body["think"] = json!(false);
    }
    body
}

/// `body: None` is the `GET /api/tags` discovery request; `Some` is `POST /api/chat`.
/// `origin` is already a validated loopback URL (`settings::local_ollama_url`), so the request
/// is pinned to it with no proxy and no scheme surprises.
fn ollama_curl_config(origin: &str, body: Option<&Value>, timeout: Duration) -> Result<String, String> {
    let path = if body.is_some() { "/api/chat" } else { "/api/tags" };
    let mut lines = vec![
        format!("url = {}", quote(&format!("{origin}{path}"))?),
        format!("request = {}", quote(if body.is_some() { "POST" } else { "GET" })?),
        "connect-timeout = 10".to_string(),
        format!("max-time = {}", timeout.as_secs()),
        "proxy = \"\"".to_string(),
        "noproxy = \"*\"".to_string(),
        "proto = \"=http,https\"".to_string(),
    ];
    if let Some(body) = body {
        let text = serde_json::to_string(body).map_err(|e| e.to_string())?;
        lines.push("header = \"content-type: application/json\"".to_string());
        lines.push(format!("data-binary = {}", quote(&text)?));
    }
    Ok(lines.join("\n") + "\n")
}

fn ollama_error(value: &Value) -> Option<String> {
    value.get("error").map(|error| {
        let message = error.as_str().map(str::to_string).unwrap_or_else(|| error.to_string());
        format!("Ollama: {}", process::truncate(&message, 300))
    })
}

/// `message.content` is itself the schema-shaped JSON text the model produced.
fn ollama_parse(value: &Value) -> Result<Value, String> {
    if let Some(error) = ollama_error(value) {
        return Err(error);
    }
    if value.get("done_reason").and_then(Value::as_str) == Some("length") {
        return Err("Ollama's answer was cut off at the token limit.".into());
    }
    if value.get("done").and_then(Value::as_bool) != Some(true) {
        return Err("Ollama's answer was incomplete.".into());
    }
    let content = value
        .pointer("/message/content")
        .and_then(Value::as_str)
        .ok_or("Ollama's reply had no message content.")?;
    serde_json::from_str(content).map_err(|e| format!("Ollama's answer was not JSON: {e}"))
}

/// Ollama proxies `…:cloud` / `…-cloud` models and any tag carrying `remote_host`/
/// `remote_model` to ollama.com — refused and hidden, never offered as "local".
fn parse_ollama_tags(value: &Value) -> Result<Vec<String>, String> {
    if let Some(error) = ollama_error(value) {
        return Err(error);
    }
    let models = value.get("models").and_then(Value::as_array).ok_or("Ollama's tags reply had no models array.")?;
    let mut names = Vec::new();
    for model in models {
        let Some(name) = model.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()) else { continue };
        let remote = model.get("remote_host").is_some() || model.get("remote_model").is_some();
        if !remote && !super::settings::is_cloud_model(name) && !names.iter().any(|n: &String| n == name) {
            names.push(name.to_string());
        }
    }
    Ok(names)
}

/// Curl's own connection-refused text (`curl: (7) …`, printed by `-sS`) is the one case with a
/// friendlier message than "curl's stderr, truncated" — everything else falls through to that.
fn ollama_stdout(output: Result<process::Output, String>, origin: &str) -> Result<Value, String> {
    let output = output?;
    if let Ok(value) = serde_json::from_str::<Value>(&output.stdout) {
        return Ok(value);
    }
    if !output.success {
        if output.stderr.contains("curl: (7)") {
            return Err(format!("Ollama is not running at {origin}."));
        }
        return Err(process::truncate(&output.stderr, 300));
    }
    Err("The reply was not JSON.".into())
}

/// `model` is in the filtered installed list when it equals one of `installed`'s entries, or
/// that entry with an implicit `:latest` tag added on either side — ASCII case-insensitive,
/// matching Ollama's own tag matching.
fn ollama_model_is_listed(model: &str, installed: &[String]) -> bool {
    installed.iter().any(|name| model.eq_ignore_ascii_case(name) || model.eq_ignore_ascii_case(&format!("{name}:latest")))
}

/// Ollama proxies a cloud-backed model to ollama.com even when its name carries no `cloud`
/// marker (`ollama cp gpt-oss:120b-cloud arch-writer` keeps the proxying under the plain name
/// `arch-writer`); `parse_ollama_tags` already hides such entries from the pick list, but a
/// hand-edited settings file or a model renamed after it was chosen could still reach here.
/// Refusing unless the model is in that same filtered list — never trusting the name alone —
/// is the one check between a stored model and ollama.com.
fn complete_ollama(curl: &Path, route: &Route, request: &Completion, max_time: Duration, cancel: &Cancel) -> Result<Value, String> {
    let installed = ollama_models(&route.ollama_url, cancel)?;
    if !ollama_model_is_listed(&route.model, &installed) {
        return Err(format!("{} is not an installed local Ollama model; choose one in AI settings.", route.model));
    }
    let body = ollama_body(&route.model, request.system, request.user, request.schema, request.max_output_tokens, request.probe);
    let config = ollama_curl_config(&route.ollama_url, Some(&body), max_time)?;
    let output = run_curl(curl, &config, outer_timeout(max_time), cancel);
    ollama_parse(&ollama_stdout(output, &route.ollama_url)?)
}

/// Names of the models installed in the Ollama at `ollama_url` (a validated loopback origin).
pub fn ollama_models(ollama_url: &str, cancel: &Cancel) -> Result<Vec<String>, String> {
    let curl = locate_curl().ok_or("curl.exe was not found; ArchGen needs it to reach Ollama.")?;
    let config = ollama_curl_config(ollama_url, None, OLLAMA_DISCOVERY_TIMEOUT)?;
    let output = run_curl(&curl, &config, outer_timeout(OLLAMA_DISCOVERY_TIMEOUT), cancel);
    parse_ollama_tags(&ollama_stdout(output, ollama_url)?)
}

// ── OpenAI ───────────────────────────────────────────────────────────────────

fn openai_body(model: &str, system: &str, user: &str, schema: &Value, max_output_tokens: u32) -> Value {
    json!({
        "model": model,
        "max_completion_tokens": max_output_tokens + CLOUD_REASONING_ALLOWANCE,
        "messages": [{"role": "developer", "content": system}, {"role": "user", "content": user}],
        "response_format": {"type": "json_schema", "json_schema": {"name": "response", "schema": schema, "strict": true}},
    })
}

fn openai_curl_config(body: &Value, api_key: &str, timeout: Duration) -> Result<String, String> {
    let text = serde_json::to_string(body).map_err(|e| e.to_string())?;
    Ok([
        format!("url = {}", quote(OPENAI_URL)?),
        "request = \"POST\"".to_string(),
        "proto = \"=https\"".to_string(),
        "connect-timeout = 10".to_string(),
        format!("max-time = {}", timeout.as_secs()),
        "header = \"content-type: application/json\"".to_string(),
        format!("header = {}", quote(&format!("authorization: Bearer {api_key}"))?),
        format!("data-binary = {}", quote(&text)?),
    ]
    .join("\n")
        + "\n")
}

fn openai_parse(value: &Value) -> Result<Value, String> {
    if let Some(error) = value.get("error") {
        let message = error.get("message").and_then(Value::as_str).unwrap_or("OpenAI returned an error.");
        return Err(process::truncate(message, 300));
    }
    match value.pointer("/choices/0/finish_reason").and_then(Value::as_str) {
        Some("length") => return Err("The answer was cut off at the token limit.".into()),
        Some("content_filter") => return Err("OpenAI declined this request.".into()),
        _ => {}
    }
    if let Some(refusal) = value.pointer("/choices/0/message/refusal").filter(|r| !r.is_null()) {
        let text = refusal.as_str().map(str::to_string).unwrap_or_else(|| refusal.to_string());
        return Err(format!("OpenAI refused: {}", process::truncate(&text, 300)));
    }
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or("OpenAI's reply had no message content.")?;
    serde_json::from_str(content).map_err(|e| format!("OpenAI's answer was not JSON: {e}"))
}

fn complete_openai(curl: &Path, route: &Route, request: &Completion, max_time: Duration, cancel: &Cancel) -> Result<Value, String> {
    let api_key = route.api_key.as_deref().ok_or("No OpenAI API key was resolved for this request.")?;
    let body = openai_body(&route.model, request.system, request.user, request.schema, request.max_output_tokens);
    let config = openai_curl_config(&body, api_key, max_time)?;
    http_json_answer(run_curl(curl, &config, outer_timeout(max_time), cancel), openai_parse)
}

// ── Gemini ───────────────────────────────────────────────────────────────────

fn gemini_url(model: &str) -> String {
    GEMINI_URL_TEMPLATE.replace("{model}", model)
}

#[derive(Clone, Copy)]
enum GeminiSchema {
    Json,
    Legacy,
}

fn gemini_body(system: &str, user: &str, schema: &Value, max_output_tokens: u32, kind: GeminiSchema) -> Value {
    let mut generation = json!({"responseMimeType": "application/json", "maxOutputTokens": max_output_tokens + CLOUD_REASONING_ALLOWANCE});
    let map = generation.as_object_mut().expect("object literal above is always an object");
    match kind {
        GeminiSchema::Json => {
            map.insert("responseJsonSchema".into(), schema.clone());
        }
        GeminiSchema::Legacy => {
            map.insert("responseSchema".into(), schema.clone());
        }
    }
    json!({
        "systemInstruction": {"parts": [{"text": system}]},
        "contents": [{"role": "user", "parts": [{"text": user}]}],
        "generationConfig": generation,
    })
}

/// Older Gemini deployments reject `responseJsonSchema`; our schemas never use the
/// `additionalProperties`/`oneOf` constructs the legacy `responseSchema` dialect cannot express,
/// so the fallback body only needs to drop the one keyword that dialect does not understand.
fn to_legacy_gemini_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| key.as_str() != "additionalProperties")
                .map(|(key, value)| (key.clone(), to_legacy_gemini_schema(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(to_legacy_gemini_schema).collect()),
        other => other.clone(),
    }
}

fn gemini_curl_config(body: &Value, model: &str, api_key: &str, timeout: Duration) -> Result<String, String> {
    let text = serde_json::to_string(body).map_err(|e| e.to_string())?;
    Ok([
        format!("url = {}", quote(&gemini_url(model))?),
        "request = \"POST\"".to_string(),
        "proto = \"=https\"".to_string(),
        "connect-timeout = 10".to_string(),
        format!("max-time = {}", timeout.as_secs()),
        "header = \"content-type: application/json\"".to_string(),
        format!("header = {}", quote(&format!("x-goog-api-key: {api_key}"))?),
        format!("data-binary = {}", quote(&text)?),
    ]
    .join("\n")
        + "\n")
}

/// `finishReason`/`blockReason` are short enum tokens (`SAFETY`, `PROHIBITED_CONTENT`, …); only
/// echo one that actually looks like one, so an unexpected provider payload can never smuggle
/// long or arbitrary text into an error message through this path (§2's 300-character quote
/// limit is still enforced by `gemini_parse`'s other branches, but a token this short never
/// needs it).
fn gemini_reason_suffix(reason: &str) -> String {
    let looks_like_a_token = !reason.is_empty() && reason.len() <= 40 && reason.bytes().all(|b| b.is_ascii_uppercase() || b == b'_');
    if looks_like_a_token {
        format!(" ({reason})")
    } else {
        String::new()
    }
}

fn gemini_parse(value: &Value) -> Result<Value, String> {
    if let Some(error) = value.get("error") {
        let message = error.get("message").and_then(Value::as_str).unwrap_or("Gemini returned an error.");
        return Err(process::truncate(message, 300));
    }
    // A blocked prompt gets HTTP 200 with no `candidates` at all — the pre-generation twin of
    // the per-candidate `finishReason` handled below.
    if let Some(reason) = value.pointer("/promptFeedback/blockReason").and_then(Value::as_str) {
        return Err(format!("Gemini declined this request{}.", gemini_reason_suffix(reason)));
    }
    match value.pointer("/candidates/0/finishReason").and_then(Value::as_str) {
        Some("MAX_TOKENS") => return Err("The answer was cut off at the token limit.".into()),
        Some(reason @ ("SAFETY" | "RECITATION" | "PROHIBITED_CONTENT" | "BLOCKLIST" | "SPII")) => {
            return Err(format!("Gemini declined this request{}.", gemini_reason_suffix(reason)));
        }
        // Any other non-STOP reason (OTHER, LANGUAGE, MALFORMED_FUNCTION_CALL, …) still means no
        // usable answer, with or without a partial text part.
        Some(reason) if reason != "STOP" => {
            return Err(format!("Gemini stopped without an answer{}.", gemini_reason_suffix(reason)));
        }
        _ => {}
    }
    let text = value
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(Value::as_str)
        .ok_or("Gemini's reply had no text.")?;
    serde_json::from_str(text).map_err(|e| format!("Gemini's answer was not JSON: {e}"))
}

fn gemini_needs_legacy_schema(stdout: &str) -> bool {
    serde_json::from_str::<Value>(stdout).ok().is_some_and(|value| {
        value.pointer("/error/code").and_then(Value::as_u64) == Some(400)
            && value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .is_some_and(|m| m.contains("responseJsonSchema"))
    })
}

fn complete_gemini(curl: &Path, route: &Route, request: &Completion, max_time: Duration, cancel: &Cancel) -> Result<Value, String> {
    let api_key = route.api_key.as_deref().ok_or("No Gemini API key was resolved for this request.")?;
    let body = gemini_body(request.system, request.user, request.schema, request.max_output_tokens, GeminiSchema::Json);
    let config = gemini_curl_config(&body, &route.model, api_key, max_time)?;
    let output = run_curl(curl, &config, outer_timeout(max_time), cancel)?;
    if gemini_needs_legacy_schema(&output.stdout) {
        let legacy_schema = to_legacy_gemini_schema(request.schema);
        let body = gemini_body(request.system, request.user, &legacy_schema, request.max_output_tokens, GeminiSchema::Legacy);
        let config = gemini_curl_config(&body, &route.model, api_key, max_time)?;
        return http_json_answer(run_curl(curl, &config, outer_timeout(max_time), cancel), gemini_parse);
    }
    http_json_answer(Ok(output), gemini_parse)
}

// ── Anthropic ────────────────────────────────────────────────────────────────

/// `output_config.format` is the current structured-output shape (the `claude-api` skill
/// confirms it; `output_format` is deprecated). No `thinking` parameter is set: omitting it
/// runs `claude-opus-5`'s adaptive thinking, which the current API allows alongside structured
/// output (the skill lists "extended thinking" among what structured outputs work with) —
/// `max_tokens` caps thinking and the answer together, which is why `CLOUD_REASONING_ALLOWANCE`
/// is added on top of the caller's answer budget below. The trivial `TEST_MAX_OUTPUT_TOKENS`
/// probe asks for low effort instead of disabling thinking outright (`{type: "disabled"}` is
/// rejected once the model's effort is `xhigh`/`max`, which ArchGen does not control): low
/// effort keeps its thinking shallow enough that the one-word answer is not competing with deep
/// reasoning for the same small cap.
fn anthropic_body(model: &str, system: &str, user: &str, schema: &Value, max_output_tokens: u32, probe: bool) -> Value {
    let mut output_config = json!({"format": {"type": "json_schema", "schema": schema}});
    if probe {
        output_config["effort"] = json!("low");
    }
    json!({
        "model": model,
        "max_tokens": max_output_tokens + CLOUD_REASONING_ALLOWANCE,
        "system": system,
        "messages": [{"role": "user", "content": user}],
        "output_config": output_config,
    })
}

fn anthropic_curl_config(body: &Value, api_key: &str, timeout: Duration) -> Result<String, String> {
    let text = serde_json::to_string(body).map_err(|e| e.to_string())?;
    Ok([
        format!("url = {}", quote(ANTHROPIC_URL)?),
        "request = \"POST\"".to_string(),
        "proto = \"=https\"".to_string(),
        "connect-timeout = 10".to_string(),
        format!("max-time = {}", timeout.as_secs()),
        "header = \"content-type: application/json\"".to_string(),
        format!("header = {}", quote(&format!("anthropic-version: {ANTHROPIC_VERSION}"))?),
        format!("header = {}", quote(&format!("x-api-key: {api_key}"))?),
        format!("data-binary = {}", quote(&text)?),
    ]
    .join("\n")
        + "\n")
}

fn anthropic_parse(value: &Value) -> Result<Value, String> {
    if value.get("type").and_then(Value::as_str) == Some("error") {
        let message = value.pointer("/error/message").and_then(Value::as_str).unwrap_or("Claude API returned an error.");
        return Err(process::truncate(message, 300));
    }
    match value.get("stop_reason").and_then(Value::as_str) {
        Some("refusal") => return Err("Claude declined this request.".into()),
        Some("max_tokens") => return Err("The answer was cut off at the token limit.".into()),
        _ => {}
    }
    let text = value
        .get("content")
        .and_then(Value::as_array)
        .and_then(|blocks| blocks.iter().find(|b| b.get("type").and_then(Value::as_str) == Some("text")))
        .and_then(|block| block.get("text"))
        .and_then(Value::as_str)
        .ok_or("Claude's reply had no text.")?;
    serde_json::from_str(text).map_err(|e| format!("Claude's answer was not JSON: {e}"))
}

fn complete_anthropic(curl: &Path, route: &Route, request: &Completion, max_time: Duration, cancel: &Cancel) -> Result<Value, String> {
    let api_key = route.api_key.as_deref().ok_or("No Claude API key was resolved for this request.")?;
    let body = anthropic_body(&route.model, request.system, request.user, request.schema, request.max_output_tokens, request.probe);
    let config = anthropic_curl_config(&body, api_key, max_time)?;
    http_json_answer(run_curl(curl, &config, outer_timeout(max_time), cancel), anthropic_parse)
}

// ── Entry point ──────────────────────────────────────────────────────────────

/// Sends one request to the API of `route.kind` and returns the JSON value it answered with.
/// `route.timeout` becomes curl's own `max-time`; `process::run`'s hard kill deadline is that
/// plus 5 s (`outer_timeout`), so curl gets the chance to report its own timeout first.
pub fn complete(route: &Route, request: &Completion, cancel: &Cancel) -> Result<Value, String> {
    let curl = locate_curl().ok_or("curl.exe was not found; ArchGen needs it to reach AI providers.")?;
    let max_time = route.timeout;
    match route.kind {
        ProviderKind::Ollama => complete_ollama(&curl, route, request, max_time, cancel),
        ProviderKind::Openai => complete_openai(&curl, route, request, max_time, cancel),
        ProviderKind::Gemini => complete_gemini(&curl, route, request, max_time, cancel),
        ProviderKind::Anthropic => complete_anthropic(&curl, route, request, max_time, cancel),
        ProviderKind::ClaudeCli | ProviderKind::CodexCli | ProviderKind::GeminiCli => {
            Err("This provider does not use HTTP.".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::TEST_MAX_OUTPUT_TOKENS;
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};

    fn schema() -> Value {
        json!({"type": "object", "additionalProperties": false, "required": ["ok"], "properties": {"ok": {"type": "boolean"}}})
    }

    // ── quote / curl location ───────────────────────────────────────────────

    #[test]
    fn quote_escapes_backslashes_and_quotes_for_curls_config_syntax() {
        assert_eq!(quote("plain").unwrap(), "\"plain\"");
        assert_eq!(quote("a\\b\"c").unwrap(), "\"a\\\\b\\\"c\"");
    }

    /// `quote` only ever needs to undo two escapes (`\\` and `\"`) for a value with no control
    /// characters — it never has to model curl's line-based config reader, because `quote`
    /// itself refuses any value that would need that (see `quote_refuses_every_control_character`
    /// below, and the real-curl tests, which are the only authority on what curl receives).
    fn curl_unescape(quoted: &str) -> String {
        let inner = &quoted[1..quoted.len() - 1];
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(escaped) = chars.next() {
                    out.push(escaped);
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn quote_round_trips_backslashes_quotes_and_a_one_megabyte_body() {
        let samples = [
            "",
            "\\",
            "\"",
            "\\\\",
            "a\\b\"c\\\"d",
            "@start-of-string #hash",
            "žluťoučký kůň 🦄 日本語",
            "mix \\ \" @ # 混合 end",
        ];
        for sample in samples {
            let quoted = quote(sample).unwrap();
            assert!(quoted.starts_with('"') && quoted.ends_with('"'), "{quoted}");
            assert_eq!(curl_unescape(&quoted), sample, "round-trip failed for {sample:?}");
        }
        let one_megabyte = "x\\\"@#日".repeat(1024 * 1024 / 7);
        assert!(one_megabyte.len() >= 1_000_000);
        assert_eq!(curl_unescape(&quote(&one_megabyte).unwrap()), one_megabyte);
    }

    /// curl reads its `--config -` text one line at a time (verified against the real
    /// System32 curl.exe: a raw `\n` inside a quoted value ends the line early and the rest is
    /// read back as a second, independent option). A value with a control character — a line
    /// feed above all — is therefore never safe to quote: `quote` refuses instead of certifying
    /// a round-trip real curl would not honour.
    #[test]
    fn quote_refuses_every_control_character() {
        for sample in ["a\nb", "a\rb", "a\r\nb", "a\tb", "a\0b", "\n"] {
            assert!(quote(sample).is_err(), "{sample:?} must be refused");
        }
    }

    /// Defence in depth: even if a caller upstream of `quote` ever let a control character
    /// through `valid_key`/`valid_model`, a config builder must still refuse it rather than
    /// hand curl a value that folds into a second config line.
    #[test]
    fn a_curl_config_builder_refuses_a_key_containing_a_newline() {
        let body = json!({"model": "m"});
        let smuggled_key = "sk-test\nproxy = \"http://attacker.example\"";
        assert!(openai_curl_config(&body, smuggled_key, Duration::from_secs(30)).is_err());
    }

    #[test]
    fn curl_is_found_on_this_windows_installation() {
        assert!(locate_curl().is_some());
    }

    #[test]
    fn the_outer_kill_deadline_is_five_seconds_past_curls_own_max_time() {
        assert_eq!(outer_timeout(Duration::from_secs(30)), Duration::from_secs(35));
    }

    // ── config text: cloud providers keep the key out of the URL ───────────

    fn assert_key_appears_once_and_never_in_the_url(config: &str, key: &str, url: &str) {
        let key_lines = config.lines().filter(|line| line.contains(key)).count();
        assert_eq!(key_lines, 1, "expected exactly one line with the key:\n{config}");
        let url_line = config.lines().find(|line| line.starts_with("url = ")).unwrap();
        assert!(url_line.contains(url), "{url_line}");
        assert!(!url_line.contains(key), "{url_line}");
        assert!(!config.to_ascii_lowercase().contains("location"), "curl must not be told to follow redirects");
        assert!(config.contains("proto = \"=https\""));
    }

    #[test]
    fn cloud_provider_configs_carry_the_key_on_one_line_and_never_in_the_url() {
        let body = json!({"model": "m"});
        let key = "sk-test-secret-key-value";
        let timeout = Duration::from_secs(30);

        assert_key_appears_once_and_never_in_the_url(&openai_curl_config(&body, key, timeout).unwrap(), key, OPENAI_URL);
        assert_key_appears_once_and_never_in_the_url(
            &gemini_curl_config(&body, "gemini-model", key, timeout).unwrap(),
            key,
            &gemini_url("gemini-model"),
        );
        assert_key_appears_once_and_never_in_the_url(&anthropic_curl_config(&body, key, timeout).unwrap(), key, ANTHROPIC_URL);
    }

    #[test]
    fn ollama_config_pins_proxy_and_protocol_and_never_writes_a_body_file() {
        let body = ollama_body("m", "sys", "user text with \"quotes\"", &schema(), 100, false);
        let config = ollama_curl_config("http://localhost:11434", Some(&body), Duration::from_secs(30)).unwrap();
        assert!(config.contains("proxy = \"\"") && config.contains("noproxy = \"*\"") && config.contains("proto = \"=http,https\""));
        assert!(config.lines().any(|l| l.starts_with("data-binary = ") && !l.contains("@")));
        assert!(config.lines().any(|l| l.starts_with("url = ") && l.ends_with("/api/chat\"")), "{config}");
        let discovery = ollama_curl_config("http://localhost:11434", None, OLLAMA_DISCOVERY_TIMEOUT).unwrap();
        assert!(discovery.contains("request = \"GET\"") && !discovery.contains("data-binary"));
        assert!(discovery.lines().any(|l| l.starts_with("url = ") && l.ends_with("/api/tags\"")), "{discovery}");
    }

    /// Every one of the four `*_curl_config` builders must ask curl for the same deadline and
    /// redirect behaviour: `rust-test-quality#11` found this pinned for the cloud helper only.
    #[test]
    fn every_curl_config_pins_the_deadline_lines_and_never_asks_for_redirects() {
        let key = "sk-test-secret-key-value";
        let timeout = Duration::from_secs(30);
        let body = json!({"model": "m"});
        let configs = [
            openai_curl_config(&body, key, timeout).unwrap(),
            gemini_curl_config(&body, "gemini-model", key, timeout).unwrap(),
            anthropic_curl_config(&body, key, timeout).unwrap(),
            ollama_curl_config("http://localhost:11434", Some(&body), timeout).unwrap(),
        ];
        for config in configs {
            assert!(config.lines().any(|l| l == "connect-timeout = 10"), "{config}");
            assert!(config.lines().any(|l| l == "max-time = 30"), "{config}");
            assert!(!config.lines().any(|l| l.starts_with("location") || l.starts_with("fail") || l.starts_with("follow")), "{config}");
        }
        let discovery = ollama_curl_config("http://localhost:11434", None, OLLAMA_DISCOVERY_TIMEOUT).unwrap();
        assert!(discovery.lines().any(|l| l == "max-time = 15"), "{discovery}");
    }

    // ── body builders ────────────────────────────────────────────────────────

    #[test]
    fn body_builders_place_system_user_schema_and_token_limit_correctly() {
        let ollama = ollama_body("m", "sys", "usr", &schema(), 42, false);
        assert_eq!(ollama["model"], "m");
        assert_eq!(ollama["messages"][0], json!({"role": "system", "content": "sys"}));
        assert_eq!(ollama["messages"][1], json!({"role": "user", "content": "usr"}));
        assert_eq!(ollama["format"], schema());
        assert_eq!(ollama["stream"], false);
        assert_eq!(ollama["options"]["num_predict"], 42);
        assert!(ollama.get("think").is_none(), "a real request leaves Ollama's own default alone");
        let ollama_probe = ollama_body("m", "sys", "usr", &schema(), TEST_MAX_OUTPUT_TOKENS, true);
        assert_eq!(ollama_probe["think"], false);

        let openai = openai_body("m", "sys", "usr", &schema(), 42);
        assert_eq!(openai["messages"][0]["role"], "developer");
        assert_eq!(openai["response_format"]["json_schema"]["schema"], schema());
        assert_eq!(openai["response_format"]["json_schema"]["strict"], true);
        assert_eq!(openai["max_completion_tokens"], 42 + CLOUD_REASONING_ALLOWANCE);

        let gemini = gemini_body("sys", "usr", &schema(), 42, GeminiSchema::Json);
        assert_eq!(gemini["systemInstruction"]["parts"][0]["text"], "sys");
        assert_eq!(gemini["contents"][0]["parts"][0]["text"], "usr");
        assert_eq!(gemini["generationConfig"]["responseJsonSchema"], schema());
        assert_eq!(gemini["generationConfig"]["maxOutputTokens"], 42 + CLOUD_REASONING_ALLOWANCE);
        let legacy = gemini_body("sys", "usr", &schema(), 42, GeminiSchema::Legacy);
        assert!(legacy["generationConfig"].get("responseSchema").is_some());
        assert!(legacy["generationConfig"].get("responseJsonSchema").is_none());

        let anthropic = anthropic_body("m", "sys", "usr", &schema(), 42, false);
        assert_eq!(anthropic["system"], "sys");
        assert_eq!(anthropic["messages"][0], json!({"role": "user", "content": "usr"}));
        assert_eq!(anthropic["output_config"]["format"]["schema"], schema());
        assert_eq!(anthropic["max_tokens"], 42 + CLOUD_REASONING_ALLOWANCE);
        assert!(anthropic["output_config"].get("effort").is_none());
        let anthropic_probe = anthropic_body("m", "sys", "usr", &schema(), TEST_MAX_OUTPUT_TOKENS, true);
        assert_eq!(anthropic_probe["output_config"]["effort"], "low");
    }

    #[test]
    fn legacy_gemini_schema_drops_only_additional_properties() {
        let strict = json!({"type": "object", "additionalProperties": false, "required": ["a"],
            "properties": {"a": {"type": "array", "items": {"type": "object", "additionalProperties": false, "properties": {"b": {"type": "string"}}}}}});
        let legacy = to_legacy_gemini_schema(&strict);
        assert!(legacy.get("additionalProperties").is_none());
        assert!(legacy["properties"]["a"]["items"].get("additionalProperties").is_none());
        assert_eq!(legacy["properties"]["a"]["items"]["properties"]["b"]["type"], "string");
    }

    #[test]
    fn gemini_needs_legacy_schema_matches_only_the_specific_400() {
        assert!(gemini_needs_legacy_schema(&json!({"error": {"code": 400, "message": "Unknown name \"responseJsonSchema\""}}).to_string()));
        assert!(!gemini_needs_legacy_schema(&json!({"error": {"code": 400, "message": "quota exceeded"}}).to_string()));
        assert!(!gemini_needs_legacy_schema(&json!({"error": {"code": 500, "message": "responseJsonSchema"}}).to_string()));
        assert!(!gemini_needs_legacy_schema("not json"));
    }

    // ── parsers: success, 4xx error body, truncated/length, refusal, garbage ──

    #[test]
    fn ollama_parse_covers_success_error_length_and_garbage() {
        assert_eq!(
            ollama_parse(&json!({"message": {"content": "{\"ok\":true}"}, "done": true})).unwrap(),
            json!({"ok": true})
        );
        assert_eq!(ollama_parse(&json!({"error": "model not found"})).unwrap_err(), "Ollama: model not found");
        assert!(ollama_parse(&json!({"message": {"content": "{"}, "done": true, "done_reason": "length"})).unwrap_err().contains("cut off"));
        assert!(ollama_parse(&json!({"message": {"content": "{\"ok\":true}"}, "done": false})).unwrap_err().contains("incomplete"));
        assert!(ollama_parse(&json!({"message": {"content": "not json"}, "done": true})).is_err());
    }

    #[test]
    fn openai_parse_covers_success_error_length_refusal_and_garbage() {
        assert_eq!(
            openai_parse(&json!({"choices": [{"finish_reason": "stop", "message": {"content": "{\"ok\":true}"}}]})).unwrap(),
            json!({"ok": true})
        );
        assert_eq!(openai_parse(&json!({"error": {"message": "invalid api key"}})).unwrap_err(), "invalid api key");
        assert!(openai_parse(&json!({"choices": [{"finish_reason": "length", "message": {"content": "{"}}]})).unwrap_err().contains("cut off"));
        assert!(openai_parse(&json!({"choices": [{"finish_reason": "stop", "message": {"refusal": "can't help"}}]})).unwrap_err().contains("can't help"));
        assert!(openai_parse(&json!({"choices": [{"finish_reason": "stop", "message": {"content": "not json"}}]})).is_err());
    }

    #[test]
    fn gemini_parse_covers_success_error_length_refusal_and_garbage() {
        assert_eq!(
            gemini_parse(&json!({"candidates": [{"finishReason": "STOP", "content": {"parts": [{"text": "{\"ok\":true}"}]}}]})).unwrap(),
            json!({"ok": true})
        );
        assert_eq!(gemini_parse(&json!({"error": {"code": 400, "message": "bad request"}})).unwrap_err(), "bad request");
        assert!(gemini_parse(&json!({"candidates": [{"finishReason": "MAX_TOKENS", "content": {"parts": [{"text": "{"}]}}]})).unwrap_err().contains("cut off"));
        assert!(gemini_parse(&json!({"candidates": [{"finishReason": "SAFETY", "content": {"parts": [{"text": ""}]}}]})).unwrap_err().contains("declined"));
        assert!(gemini_parse(&json!({"candidates": [{"finishReason": "STOP", "content": {"parts": [{"text": "not json"}]}}]})).is_err());
    }

    /// `providers-real-world#8`: a prompt-level block never returns `candidates` at all, and
    /// several per-candidate finish reasons carry no text (or only a fragment) — none of those
    /// were distinguished from "the model just didn't answer".
    #[test]
    fn gemini_parse_names_a_prompt_block_and_every_no_text_finish_reason() {
        let prompt_block = gemini_parse(&json!({"promptFeedback": {"blockReason": "PROHIBITED_CONTENT"}})).unwrap_err();
        assert!(prompt_block.contains("declined") && prompt_block.contains("PROHIBITED_CONTENT"), "{prompt_block}");

        let no_content = gemini_parse(&json!({"candidates": [{"finishReason": "PROHIBITED_CONTENT", "index": 0}]})).unwrap_err();
        assert!(no_content.contains("declined") && no_content.contains("PROHIBITED_CONTENT"), "{no_content}");

        // A non-declining, non-STOP reason with a fragment of text is still "no usable answer",
        // never a raw JSON-parse error.
        let partial = gemini_parse(&json!({"candidates": [{"finishReason": "OTHER", "content": {"parts": [{"text": "{\"sections\""}]}}]})).unwrap_err();
        assert!(partial.contains("stopped without an answer") && partial.contains("OTHER"), "{partial}");
    }

    #[test]
    fn anthropic_parse_covers_success_error_length_refusal_and_garbage() {
        assert_eq!(
            anthropic_parse(&json!({"type": "message", "stop_reason": "end_turn", "content": [{"type": "text", "text": "{\"ok\":true}"}]})).unwrap(),
            json!({"ok": true})
        );
        assert_eq!(
            anthropic_parse(&json!({"type": "error", "error": {"type": "invalid_request_error", "message": "bad request"}})).unwrap_err(),
            "bad request"
        );
        assert!(anthropic_parse(&json!({"type": "message", "stop_reason": "max_tokens", "content": [{"type": "text", "text": "{"}]})).unwrap_err().contains("cut off"));
        assert!(anthropic_parse(&json!({"type": "message", "stop_reason": "refusal", "content": []})).unwrap_err().contains("declined"));
        assert!(anthropic_parse(&json!({"type": "message", "stop_reason": "end_turn", "content": [{"type": "text", "text": "not json"}]})).is_err());
    }

    #[test]
    fn ollama_stdout_maps_curl_exit_7_to_a_not_running_sentence_on_empty_stdout() {
        let refused = Ok(process::Output {
            stdout: String::new(),
            stderr: "curl: (7) Failed to connect to localhost port 11434 after 2 ms: Could not connect to server".into(),
            success: false,
        });
        assert_eq!(
            ollama_stdout(refused, "http://localhost:11434").unwrap_err(),
            "Ollama is not running at http://localhost:11434."
        );

        // Any other failure with empty stdout falls back to curl's own (truncated) stderr.
        let other = Ok(process::Output { stdout: String::new(), stderr: "curl: (6) Could not resolve host".into(), success: false });
        assert!(ollama_stdout(other, "http://localhost:11434").unwrap_err().contains("(6)"));
    }

    #[test]
    fn parse_ollama_tags_hides_cloud_and_remote_models_and_deduplicates() {
        let tags = json!({"models": [
            {"name": "qwen3.8:27b"}, {"name": "qwen3.8:27b"}, {"name": "llama3:cloud"},
            {"name": "llama3:70b-cloud"}, {"name": "remote-model", "remote_host": "https://ollama.com"}, {"name": ""},
        ]});
        assert_eq!(parse_ollama_tags(&tags).unwrap(), vec!["qwen3.8:27b".to_string()]);
        assert_eq!(parse_ollama_tags(&json!({"error": "down"})).unwrap_err(), "Ollama: down");
        assert!(parse_ollama_tags(&json!({"no_models_key": true})).is_err());
    }

    /// `secrets-and-injection#5`: a model is only ever sent to when discovery's own filtered
    /// list (already stripped of cloud/remote entries by `parse_ollama_tags`) names it, matched
    /// case-insensitively and with an implicit `:latest` on either side.
    #[test]
    fn ollama_model_is_listed_matches_case_insensitively_and_the_implicit_latest_tag() {
        let installed = vec!["qwen3.8:27b".to_string(), "arch-writer".to_string()];
        assert!(ollama_model_is_listed("qwen3.8:27b", &installed));
        assert!(ollama_model_is_listed("QWEN3.8:27B", &installed));
        assert!(ollama_model_is_listed("arch-writer:latest", &installed));
        assert!(ollama_model_is_listed("Arch-Writer:LATEST", &installed));
        assert!(!ollama_model_is_listed("arch-writer:7b", &installed));
        assert!(!ollama_model_is_listed("unknown-model", &installed));
        assert!(!ollama_model_is_listed("qwen3.8:27b", &[]));
    }

    // ── §2's 300-character quote limit on provider-supplied error text ─────

    #[test]
    fn every_parser_truncates_provider_supplied_error_text_to_300_characters() {
        let long = "x".repeat(1000);
        let truncated_len = process::truncate(&long, 300).chars().count();

        assert_eq!(ollama_error(&json!({"error": &long})).unwrap().chars().count(), "Ollama: ".len() + truncated_len);
        assert_eq!(openai_parse(&json!({"error": {"message": &long}})).unwrap_err().chars().count(), truncated_len);
        assert_eq!(gemini_parse(&json!({"error": {"message": &long}})).unwrap_err().chars().count(), truncated_len);
        assert_eq!(anthropic_parse(&json!({"type": "error", "error": {"message": &long}})).unwrap_err().chars().count(), truncated_len);

        let refusal = openai_parse(&json!({"choices": [{"finish_reason": "stop", "message": {"refusal": &long}}]})).unwrap_err();
        assert_eq!(refusal.chars().count(), "OpenAI refused: ".len() + truncated_len);
    }

    // ── real curl against a local TCP listener (Ollama shape) ──────────────

    fn read_request(stream: &mut std::net::TcpStream) -> (String, String) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut headers = String::new();
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                content_length = rest.trim().parse().unwrap_or(0);
            }
            headers.push_str(&line);
        }
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).unwrap();
        (headers, String::from_utf8(body).unwrap())
    }

    fn respond(stream: &mut std::net::TcpStream, status_line: &str, body: &str) {
        let response =
            format!("HTTP/1.1 {status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    }

    /// Answers the mandatory `GET /api/tags` discovery request (`secrets-and-injection#5`:
    /// `complete_ollama` refuses any model discovery does not list) before the exchange under
    /// test, so every real-curl test below only has to describe its own request/response.
    fn serve_ollama_discovery_then(listener: std::net::TcpListener, installed_model: &str) -> std::net::TcpStream {
        let (mut tags, _) = listener.accept().unwrap();
        let _ = read_request(&mut tags);
        respond(&mut tags, "200 OK", &json!({"models": [{"name": installed_model}]}).to_string());
        let (stream, _) = listener.accept().unwrap();
        stream
    }

    #[test]
    fn real_curl_delivers_the_exact_body_with_no_authorization_header() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let mut stream = serve_ollama_discovery_then(listener, "test-model");
            let (headers, body) = read_request(&mut stream);
            respond(&mut stream, "200 OK", &json!({"message": {"content": "{\"ok\":true}"}, "done": true}).to_string());
            (headers, body)
        });

        let tricky = "quotes \" backslashes \\ newline\ninside, cr\rhere, tab\there, @start-of-string, #hash, non-ascii žluťoučký 🦄";
        let route = Route {
            kind: ProviderKind::Ollama,
            model: "test-model".into(),
            api_key: None,
            ollama_url: format!("http://127.0.0.1:{}", addr.port()),
            timeout: Duration::from_secs(10),
        };
        let request = Completion { system: "system prompt", user: tricky, schema: &schema(), max_output_tokens: 32, probe: false };
        // `super::super::complete` is `ai::complete`, the single production entry point that
        // applies the schema re-check and key redaction (`rust-test-quality#1`) — `complete`
        // unqualified would resolve to this module's own `http::complete`, which bypasses both.
        let answer = super::super::complete(&route, &request, &Cancel::new()).unwrap();
        assert_eq!(answer, json!({"ok": true}));

        let (headers, body) = worker.join().unwrap();
        assert!(headers.starts_with("POST /api/chat HTTP/1.1"), "{headers}");
        assert!(headers.to_ascii_lowercase().contains("content-type: application/json"), "{headers}");
        assert!(!headers.to_ascii_lowercase().contains("authorization"), "{headers}");
        let sent: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(sent, ollama_body("test-model", "system prompt", tricky, &schema(), 32, false), "the server must receive exactly the body ollama_body built");
    }

    #[test]
    fn real_curl_delivers_a_one_megabyte_body_byte_exact() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let mut stream = serve_ollama_discovery_then(listener, "test-model");
            let (_, body) = read_request(&mut stream);
            respond(&mut stream, "200 OK", &json!({"message": {"content": "{\"ok\":true}"}, "done": true}).to_string());
            body
        });

        let big = "payload \\\"@#日本語\t\r\n".repeat(1024 * 1024 / 20);
        assert!(big.len() >= 1_000_000, "{}", big.len());
        let route = Route {
            kind: ProviderKind::Ollama,
            model: "test-model".into(),
            api_key: None,
            ollama_url: format!("http://127.0.0.1:{}", addr.port()),
            timeout: Duration::from_secs(20),
        };
        let request = Completion { system: "s", user: &big, schema: &schema(), max_output_tokens: 8, probe: false };
        super::super::complete(&route, &request, &Cancel::new()).unwrap();

        let body = worker.join().unwrap();
        let sent: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(sent["messages"][1]["content"].as_str().unwrap(), big, "a 1 MB body must arrive byte-exact");
    }

    #[test]
    fn real_curl_surfaces_a_404_json_error_body_even_though_curl_exits_zero() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let mut stream = serve_ollama_discovery_then(listener, "missing-model");
            let _ = read_request(&mut stream);
            respond(&mut stream, "404 Not Found", &json!({"error": "model not found"}).to_string());
        });

        let route = Route {
            kind: ProviderKind::Ollama,
            model: "missing-model".into(),
            api_key: None,
            ollama_url: format!("http://127.0.0.1:{}", addr.port()),
            timeout: Duration::from_secs(10),
        };
        let request = Completion { system: "s", user: "u", schema: &schema(), max_output_tokens: 8, probe: false };
        let error = super::super::complete(&route, &request, &Cancel::new()).unwrap_err();
        // `rust-test-quality#7`: assert before joining. `unwrap_err()` above accepts any error,
        // including one from curl never having connected at all; asserting first means a
        // regression that produces that outcome panics here — with a useful message — instead
        // of this thread going on to block forever on a `join` the worker can never satisfy.
        assert!(error.contains("model not found"), "{error}");
        worker.join().unwrap();
    }

    /// `rust-test-quality#1`: a hermetic in-process case for the guard `ai::complete` (mod.rs)
    /// applies after every provider call — the schema re-check — reached the same way the three
    /// tests above reach `ai::complete`, so a regression that drops `.and_then(schema::check)`
    /// turns this red.
    #[test]
    fn ai_complete_rejects_an_ollama_answer_that_fails_the_schema_re_check() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let mut stream = serve_ollama_discovery_then(listener, "test-model");
            let _ = read_request(&mut stream);
            respond(&mut stream, "200 OK", &json!({"message": {"content": "{\"ok\":\"yes\"}"}, "done": true}).to_string());
        });

        let route = Route {
            kind: ProviderKind::Ollama,
            model: "test-model".into(),
            api_key: None,
            ollama_url: format!("http://127.0.0.1:{}", addr.port()),
            timeout: Duration::from_secs(10),
        };
        let request = Completion { system: "s", user: "u", schema: &schema(), max_output_tokens: 8, probe: false };
        let error = super::super::complete(&route, &request, &Cancel::new()).unwrap_err();
        assert!(error.contains("did not have the expected shape") && error.contains("ok must be boolean"), "{error}");
        worker.join().unwrap();
    }

    /// `rust-test-quality#1`'s other guard: `ai::complete` redacts every known key from every
    /// error, even one whose route is Ollama's (a route can carry an `api_key` regardless of
    /// provider — `redact` does not special-case it).
    #[test]
    fn ai_complete_redacts_the_key_from_an_error_the_provider_echoed_back() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let mut stream = serve_ollama_discovery_then(listener, "test-model");
            let _ = read_request(&mut stream);
            respond(&mut stream, "404 Not Found", &json!({"error": "bad key sk-live-1234567890"}).to_string());
        });

        let route = Route {
            kind: ProviderKind::Ollama,
            model: "test-model".into(),
            api_key: Some("sk-live-1234567890".into()),
            ollama_url: format!("http://127.0.0.1:{}", addr.port()),
            timeout: Duration::from_secs(10),
        };
        let request = Completion { system: "s", user: "u", schema: &schema(), max_output_tokens: 8, probe: false };
        let error = super::super::complete(&route, &request, &Cancel::new()).unwrap_err();
        assert!(error.contains("[redacted]") && !error.contains("sk-live"), "{error}");
        worker.join().unwrap();
    }

    /// `secrets-and-injection#5`: a model with no `cloud`/`-cloud` tag but a `remote_host` in
    /// its tags entry (e.g. `ollama cp gpt-oss:120b-cloud arch-writer`) must still be refused —
    /// the chat request must never be sent — because discovery's filtered list never lists it.
    #[test]
    fn real_curl_refuses_a_model_thats_not_in_the_filtered_installed_list_before_ever_posting() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_request(&mut stream);
            respond(&mut stream, "200 OK", &json!({"models": [{"name": "arch-writer", "remote_host": "https://ollama.com"}]}).to_string());
            // No second `accept`: a chat POST must never arrive.
        });

        let route = Route {
            kind: ProviderKind::Ollama,
            model: "arch-writer".into(),
            api_key: None,
            ollama_url: format!("http://127.0.0.1:{}", addr.port()),
            timeout: Duration::from_secs(10),
        };
        let request = Completion { system: "s", user: "u", schema: &schema(), max_output_tokens: 8, probe: false };
        let error = super::super::complete(&route, &request, &Cancel::new()).unwrap_err();
        assert!(error.contains("arch-writer") && error.contains("not an installed local Ollama model"), "{error}");
        worker.join().unwrap();
    }

    // ── live checks (network, real providers) ───────────────────────────────

    #[test]
    #[ignore]
    fn live_ollama() {
        let url = "http://localhost:11434";
        // Contract §8/§9: a live check passes with a message when the provider is unavailable
        // and only fails on a broken integration — never on "this machine happens not to have
        // Ollama running" or "this machine's Ollama has a different model installed".
        let installed = match ollama_models(url, &Cancel::new()) {
            Ok(models) if !models.is_empty() => models,
            Ok(_) => {
                println!("skipping live_ollama: no local Ollama model is installed");
                return;
            }
            Err(error) => {
                println!("skipping live_ollama: {error}");
                return;
            }
        };
        // Prefer an explicit override, then the documented development model, then whatever
        // discovery listed first — never assume the exact model this developer's machine has.
        let model = std::env::var("ARCHGEN_LIVE_OLLAMA_MODEL")
            .ok()
            .filter(|m| installed.iter().any(|i| i.eq_ignore_ascii_case(m)))
            .or_else(|| installed.iter().find(|m| m.starts_with("qwen3.8")).cloned())
            .unwrap_or_else(|| installed[0].clone());
        println!("live_ollama: using {model}");

        let route = Route {
            kind: ProviderKind::Ollama,
            model,
            api_key: None,
            ollama_url: url.into(),
            // The first request against a large model can take a while to load; give it room.
            timeout: Duration::from_secs(540),
        };
        let request = Completion {
            system: "Reply with exactly one JSON object of the required shape and nothing else.",
            user: "Return {\"ok\": true}.",
            schema: &schema(),
            max_output_tokens: TEST_MAX_OUTPUT_TOKENS,
            probe: true,
        };
        let value = super::super::complete(&route, &request, &Cancel::new()).unwrap();
        assert_eq!(value, json!({"ok": true}));
    }

    /// Resolves `kind`'s key the same way `route()` would — env vars in `key_variables()`
    /// priority order, trimmed, checked with `valid_key` — so a machine with a usable key under
    /// any of them (not just the first-listed variable) runs the check, and one with none skips
    /// with a message instead of failing (contract §8/§9).
    fn resolve_live_key(kind: ProviderKind) -> Option<String> {
        kind.key_variables().iter().find_map(|&variable| {
            let raw = std::env::var(variable).ok()?;
            let trimmed = raw.trim();
            (!trimmed.is_empty() && super::super::settings::valid_key(trimmed)).then(|| trimmed.to_string())
        })
    }

    fn live_cloud_check(kind: ProviderKind) {
        let Some(key) = resolve_live_key(kind) else {
            println!("skipping live_{}: no usable key in {:?}", kind.id(), kind.key_variables());
            return;
        };
        let route = Route { kind, model: kind.default_model().into(), api_key: Some(key), ollama_url: String::new(), timeout: Duration::from_secs(60) };
        let request = Completion {
            system: "Reply with exactly one JSON object of the required shape and nothing else.",
            user: "Return {\"ok\": true}.",
            schema: &schema(),
            max_output_tokens: TEST_MAX_OUTPUT_TOKENS,
            probe: true,
        };
        let value = super::super::complete(&route, &request, &Cancel::new()).unwrap();
        assert_eq!(value, json!({"ok": true}));
    }

    #[test]
    #[ignore]
    fn live_openai() {
        live_cloud_check(ProviderKind::Openai);
    }

    #[test]
    #[ignore]
    fn live_gemini() {
        live_cloud_check(ProviderKind::Gemini);
    }

    #[test]
    #[ignore]
    fn live_anthropic() {
        live_cloud_check(ProviderKind::Anthropic);
    }
}
