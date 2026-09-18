//! Which assistant answers, with which model, and where its key comes from.
//! Stored per user in the app-data folder — never in the project folder, which is in Git.
//! See docs/AI-REWORK-DESIGN.md.
use super::{process::Cancel, Ai, Completion, ProviderKind, Route};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};
use tauri::State;

pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 600;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub version: u32,
    pub provider: ProviderKind,
    /// Provider id → model. A missing entry means `ProviderKind::default_model`.
    pub models: BTreeMap<String, String>,
    /// Provider id → key protected by `secret::protect`. Never leaves this module in plain text
    /// except inside a `Route`.
    pub keys: BTreeMap<String, String>,
    pub ollama_url: String,
    pub timeout_seconds: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            provider: ProviderKind::Ollama,
            models: BTreeMap::new(),
            keys: BTreeMap::new(),
            ollama_url: DEFAULT_OLLAMA_URL.into(),
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
        }
    }
}

/// What the settings dialog shows for one provider. Never carries a key.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub id: String,
    pub label: String,
    pub local: bool,
    pub cli: bool,
    pub recipient: String,
    /// The configured model, or the default when none is configured; may be empty.
    pub model: String,
    pub default_model: String,
    /// Ready to try: CLI found on PATH, key present, or Ollama model chosen.
    pub available: bool,
    /// One sentence the user can act on when `available` is false; otherwise what will be used.
    pub detail: String,
    /// "env" | "settings" | "none" | "not_needed"
    pub key_source: String,
    /// `ProviderKind::notice`, shown together with the consent text.
    pub notice: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    pub provider: String,
    pub providers: Vec<ProviderStatus>,
    pub ollama_url: String,
    pub timeout_seconds: u64,
}

/// A missing file is the default settings; an unreadable one is an error, never silently replaced.
pub fn load(path: &Path) -> Result<Settings, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| {
            format!("The AI settings file is damaged and was left untouched: {e}. Fix or delete {}.", path.display())
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(format!("The AI settings file could not be read: {e}")),
    }
}

/// Every writer in this process takes turns so two saves in flight never interleave their
/// temp-file-plus-rename and leave the file holding a mix of both.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Atomic write (temporary file + rename) of the whole settings file.
pub fn store(path: &Path, settings: &Settings) -> Result<(), String> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())? + "\n";
    let parent = path.parent().ok_or("Invalid AI settings path.")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("creating {}: {e}", parent.display()))?;
    let mut name = path.file_name().ok_or("Invalid AI settings path.")?.to_owned();
    name.push(format!(".tmp-{}", std::process::id()));
    let temp = parent.join(name);
    let result = std::fs::write(&temp, &text).and_then(|_| std::fs::rename(&temp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(|e| format!("writing AI settings: {e}"))
}

/// A model name that is safe as a process argument and inside a URL path.
pub fn valid_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 120
        && !model.starts_with('-')
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-'))
}

/// A key is printable ASCII without spaces, 1..=512 bytes: it goes into one line of curl's
/// config, where a line break or quote would become a second option. Applies to keys from
/// the environment as well as from the dialog.
pub fn valid_key(key: &str) -> bool {
    (1..=512).contains(&key.len()) && key.bytes().all(|b| (0x21..=0x7E).contains(&b))
}

/// Ollama proxies models tagged `cloud` / `…-cloud` to ollama.com: not local, so not allowed here.
pub fn is_cloud_model(model: &str) -> bool {
    let tag = model.rsplit_once(':').map_or("", |(_, tag)| tag).to_ascii_lowercase();
    tag == "cloud" || tag.ends_with("-cloud")
}

const REJECTED_OLLAMA_URL: &str =
    "Ollama is local: the URL must be http://localhost, http://127.0.0.1 or http://[::1], \
     with an optional :port and nothing after it but a single trailing /.";

/// Rejects, never repairs: exactly `http://` or `https://`, host `localhost`, `127.0.0.1`
/// or `[::1]`, an optional `:1..=65535`, one optional trailing `/`. Returns the origin.
pub fn local_ollama_url(url: &str) -> Result<String, String> {
    if url.chars().any(|c| c.is_control()) {
        return Err(REJECTED_OLLAMA_URL.into());
    }
    let (scheme, rest) = url.split_once("://").ok_or(REJECTED_OLLAMA_URL)?;
    if scheme != "http" && scheme != "https" {
        return Err(REJECTED_OLLAMA_URL.into());
    }
    let split = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(split);
    if !(tail.is_empty() || tail == "/") || authority.contains('@') {
        return Err(REJECTED_OLLAMA_URL.into());
    }
    let (host, port) = if let Some(inside) = authority.strip_prefix('[') {
        let close = inside.find(']').ok_or(REJECTED_OLLAMA_URL)?;
        (format!("[{}]", &inside[..close]), &inside[close + 1..])
    } else {
        match authority.split_once(':') {
            Some((host, _)) => (host.to_string(), &authority[host.len()..]),
            None => (authority.to_string(), ""),
        }
    };
    if host != "localhost" && host != "127.0.0.1" && host != "[::1]" {
        return Err(REJECTED_OLLAMA_URL.into());
    }
    if !port.is_empty() {
        let digits = port.strip_prefix(':').ok_or(REJECTED_OLLAMA_URL)?;
        let valid_port = !digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
            && digits.parse::<u16>().is_ok_and(|p| p >= 1);
        if !valid_port {
            return Err(REJECTED_OLLAMA_URL.into());
        }
    }
    Ok(format!("{scheme}://{authority}"))
}

/// Where a resolved API key came from, so `status` can report it and `find_key` only has to
/// decide once which environment variable, if any, it used.
enum KeySource {
    Env(&'static str),
    Settings,
}

/// Environment first (first variable in `kind.key_variables()` that is set to a non-blank
/// value), then the settings file. A key that fails `valid_key` from the environment is a hard
/// error naming the variable; one that fails from the settings file (corrupt or hand-edited,
/// since `ai_set_key` only ever stores a value that already passed `valid_key`) counts as none.
fn find_key(
    settings: &Settings,
    kind: ProviderKind,
    env: &impl Fn(&str) -> Option<String>,
) -> Result<(String, KeySource), String> {
    for &variable in kind.key_variables() {
        if let Some(raw) = env(variable) {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            return if valid_key(trimmed) {
                Ok((trimmed.to_string(), KeySource::Env(variable)))
            } else {
                Err(format!("{variable} contains whitespace or control characters."))
            };
        }
    }
    if let Some(key) = settings.keys.get(kind.id()).and_then(|stored| super::secret::unprotect(stored)) {
        if valid_key(&key) {
            return Ok((key, KeySource::Settings));
        }
    }
    Err(no_key_message(kind))
}

fn no_key_message(kind: ProviderKind) -> String {
    match kind {
        ProviderKind::Openai => "Set an OpenAI API key in AI settings or start ArchGen with OPENAI_API_KEY.".into(),
        ProviderKind::Gemini => "Set a Gemini API key in AI settings or start ArchGen with GEMINI_API_KEY.".into(),
        ProviderKind::Anthropic => {
            "Set an Anthropic API key in AI settings or start ArchGen with ANTHROPIC_API_KEY.".into()
        }
        _ => format!("{} does not use an API key.", kind.label()),
    }
}

/// The human name used in "Uses the … login on this computer." — the CLI's own product name,
/// not ArchGen's "<Product> CLI" label (which the *unavailable* message uses instead).
fn login_name(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::ClaudeCli => "Claude Code",
        ProviderKind::CodexCli => "Codex",
        ProviderKind::GeminiCli => "Gemini",
        _ => kind.label(),
    }
}

/// `settings::route`'s real body, taking the environment and the CLI locator as parameters so
/// tests never touch the process environment or the real PATH (see the module contract).
/// `route` below is the only caller that passes the real ones.
fn route_with(
    settings: &Settings,
    kind: ProviderKind,
    env: &impl Fn(&str) -> Option<String>,
    locate: &impl Fn(ProviderKind) -> Option<PathBuf>,
) -> Result<Route, String> {
    let timeout = Duration::from_secs(settings.timeout_seconds.clamp(30, 3600));
    if kind == ProviderKind::Ollama {
        let ollama_url = local_ollama_url(&settings.ollama_url)?;
        let model = settings.models.get(kind.id()).cloned().unwrap_or_default();
        if model.is_empty() || !valid_model(&model) {
            return Err("Choose an Ollama model in AI settings.".into());
        }
        if is_cloud_model(&model) {
            return Err("Ollama cloud models send content to ollama.com; choose an installed local model.".into());
        }
        return Ok(Route { kind, model, api_key: None, ollama_url, timeout });
    }
    let ollama_url = settings.ollama_url.clone();
    if kind.is_cli() {
        locate(kind).ok_or_else(|| format!("{} is not installed or not on PATH.", kind.label()))?;
        let model = settings.models.get(kind.id()).cloned().unwrap_or_default();
        if !model.is_empty() && !valid_model(&model) {
            return Err(format!("\"{model}\" is not a valid model name."));
        }
        return Ok(Route { kind, model, api_key: None, ollama_url, timeout });
    }
    let configured = settings.models.get(kind.id()).cloned().unwrap_or_default();
    if !configured.is_empty() && !valid_model(&configured) {
        return Err(format!("\"{configured}\" is not a valid model name."));
    }
    let model = if configured.is_empty() { kind.default_model().to_string() } else { configured };
    let (api_key, _) = find_key(settings, kind, env)?;
    Ok(Route { kind, model, api_key: Some(api_key), ollama_url, timeout })
}

/// Resolves everything a request to `kind` needs, or says what the user must do first.
pub fn route(settings: &Settings, kind: ProviderKind) -> Result<Route, String> {
    route_with(settings, kind, &|name| std::env::var(name).ok(), &super::cli::locate)
}

fn configured_model(settings: &Settings, kind: ProviderKind) -> String {
    let stored = settings.models.get(kind.id()).cloned().unwrap_or_default();
    if stored.is_empty() {
        kind.default_model().to_string()
    } else {
        stored
    }
}

/// Availability and its detail sentence come from `route_with` itself — the same function
/// `route()` calls to actually resolve a request — so `status()` can never say `available:true`
/// for a stored value `route()` would then reject (`contract-completeness#5`): a hand-edited
/// `--dangerously-skip-permissions` CLI "model" or a `"gpt 4o"` API model now shows up here too,
/// not only at request time.
fn provider_status(
    settings: &Settings,
    kind: ProviderKind,
    env: &impl Fn(&str) -> Option<String>,
    locate: &impl Fn(ProviderKind) -> Option<PathBuf>,
) -> ProviderStatus {
    let model = configured_model(settings, kind);
    // The key source must stay accurate even when the model turns out to be invalid — the
    // settings dialog uses it to decide whether "Remove key" is enabled — so it is derived
    // independently of whether `route_with` below succeeds.
    let key = kind.needs_key().then(|| find_key(settings, kind, env));
    let key_source = match &key {
        None => "not_needed",
        Some(Ok((_, KeySource::Env(_)))) => "env",
        Some(Ok((_, KeySource::Settings))) => "settings",
        Some(Err(_)) => "none",
    };
    let (available, detail) = match route_with(settings, kind, env, locate) {
        Ok(route) if kind == ProviderKind::Ollama => (true, format!("Uses {} on this computer.", route.model)),
        Ok(_) if kind.is_cli() => (true, format!("Uses the {} login on this computer.", login_name(kind))),
        Ok(route) => match &key {
            Some(Ok((_, KeySource::Env(variable)))) => (true, format!("Uses {} with the key from {variable}.", route.model)),
            _ => (true, format!("Uses {} with the stored API key.", route.model)),
        },
        Err(reason) => (false, reason),
    };
    ProviderStatus {
        id: kind.id().into(),
        label: kind.label().into(),
        local: kind.is_local(),
        cli: kind.is_cli(),
        recipient: kind.recipient().into(),
        model,
        default_model: kind.default_model().into(),
        available,
        detail,
        key_source: key_source.into(),
        notice: kind.notice().into(),
    }
}

fn status_with(settings: &Settings, env: impl Fn(&str) -> Option<String>, locate: impl Fn(ProviderKind) -> Option<PathBuf>) -> AiStatus {
    AiStatus {
        provider: settings.provider.id().into(),
        providers: ProviderKind::ALL.iter().map(|&kind| provider_status(settings, kind, &env, &locate)).collect(),
        ollama_url: settings.ollama_url.clone(),
        timeout_seconds: settings.timeout_seconds.clamp(30, 3600),
    }
}

pub fn status(settings: &Settings) -> AiStatus {
    status_with(settings, |name| std::env::var(name).ok(), super::cli::locate)
}

/// Validates every argument, then applies them all — so a rejected call never partially writes.
/// `model` targets `provider` when given, else the settings' current provider (i.e. "the current
/// one" the doc comment on `ai_configure` promises); an empty `model` removes the override.
fn apply_configure(
    settings: &mut Settings,
    provider: Option<String>,
    model: Option<String>,
    ollama_url: Option<String>,
    timeout_seconds: Option<u64>,
) -> Result<(), String> {
    let new_provider = match &provider {
        Some(id) => ProviderKind::parse(id).ok_or_else(|| format!("\"{id}\" is not a known AI provider."))?,
        None => settings.provider,
    };
    let model_update = match &model {
        None => None,
        Some(raw) => {
            let trimmed = raw.trim().to_string();
            if trimmed.is_empty() {
                Some(None)
            } else if !valid_model(&trimmed) {
                return Err(format!("\"{trimmed}\" is not a valid model name."));
            } else if new_provider == ProviderKind::Ollama && is_cloud_model(&trimmed) {
                return Err("Ollama cloud models send content to ollama.com; choose an installed local model.".into());
            } else {
                Some(Some(trimmed))
            }
        }
    };
    let new_ollama_url = match &ollama_url {
        None => None,
        Some(raw) if raw.trim().is_empty() => Some(DEFAULT_OLLAMA_URL.to_string()),
        Some(raw) => Some(local_ollama_url(raw)?),
    };
    let new_timeout = timeout_seconds.map(|seconds| seconds.clamp(30, 3600));

    settings.provider = new_provider;
    if let Some(update) = model_update {
        match update {
            Some(new_model) => {
                settings.models.insert(new_provider.id().into(), new_model);
            }
            None => {
                settings.models.remove(new_provider.id());
            }
        }
    }
    if let Some(url) = new_ollama_url {
        settings.ollama_url = url;
    }
    if let Some(timeout) = new_timeout {
        settings.timeout_seconds = timeout;
    }
    Ok(())
}

fn apply_set_key(settings: &mut Settings, kind: ProviderKind, key: &str) -> Result<(), String> {
    if !kind.needs_key() {
        return Err(format!("{} does not use an API key.", kind.label()));
    }
    if key.is_empty() {
        settings.keys.remove(kind.id());
    } else if !valid_key(key) {
        return Err("API keys are printable ASCII text with no spaces, at most 512 bytes.".into());
    } else {
        settings.keys.insert(kind.id().into(), super::secret::protect(key)?);
    }
    Ok(())
}

#[tauri::command]
pub fn ai_status(ai: State<'_, Ai>) -> Result<AiStatus, String> {
    Ok(status(&load(ai.settings_path())?))
}

/// The pure body of `ai_configure`, taking the settings path directly so tests exercise the
/// exact sequence the command runs (load, validate-and-apply, store, re-read) against a temp
/// file instead of replaying a hand-composed approximation of it.
fn configure_at(
    path: &Path,
    provider: Option<String>,
    model: Option<String>,
    ollama_url: Option<String>,
    timeout_seconds: Option<u64>,
) -> Result<AiStatus, String> {
    let mut settings = load(path)?;
    apply_configure(&mut settings, provider, model, ollama_url, timeout_seconds)?;
    store(path, &settings)?;
    Ok(status(&settings))
}

/// Every argument is optional; `model` applies to `provider` when given, else to the current one.
/// An empty `model` removes the override.
#[tauri::command]
pub fn ai_configure(
    ai: State<'_, Ai>,
    provider: Option<String>,
    model: Option<String>,
    ollama_url: Option<String>,
    timeout_seconds: Option<u64>,
) -> Result<AiStatus, String> {
    configure_at(ai.settings_path(), provider, model, ollama_url, timeout_seconds)
}

/// The pure body of `ai_set_key`, taking the settings path directly (see `configure_at`).
fn set_key_at(path: &Path, provider: String, key: &str) -> Result<AiStatus, String> {
    let kind = ProviderKind::parse(&provider).ok_or_else(|| format!("\"{provider}\" is not a known AI provider."))?;
    let mut settings = load(path)?;
    apply_set_key(&mut settings, kind, key)?;
    store(path, &settings)?;
    Ok(status(&settings))
}

/// Stores the key protected; an empty `key` removes it. The key is never returned.
#[tauri::command]
pub fn ai_set_key(ai: State<'_, Ai>, provider: String, key: String) -> Result<AiStatus, String> {
    set_key_at(ai.settings_path(), provider, &key)
}

#[tauri::command]
pub async fn ai_ollama_models(ai: State<'_, Ai>) -> Result<Vec<String>, String> {
    let origin = local_ollama_url(&load(ai.settings_path())?.ollama_url)?;
    tokio::task::spawn_blocking(move || super::http::ollama_models(&origin, &Cancel::new()))
        .await
        .map_err(|e| e.to_string())?
}

/// A minimal real request to `provider`; returns a short success sentence naming the model.
#[tauri::command]
pub async fn ai_test_provider(ai: State<'_, Ai>, provider: String) -> Result<String, String> {
    let kind = ProviderKind::parse(&provider).ok_or_else(|| format!("\"{provider}\" is not a known AI provider."))?;
    let route = ai.route_for(kind)?;
    let model = if route.model.is_empty() { kind.label().to_string() } else { route.model.clone() };
    tokio::task::spawn_blocking(move || {
        let schema = serde_json::json!({
            "type": "object", "additionalProperties": false, "required": ["ok"],
            "properties": {"ok": {"type": "boolean"}},
        });
        let request = Completion {
            system: "Reply with exactly one JSON object of the required shape and nothing else.",
            user: "Return {\"ok\": true}.",
            schema: &schema,
            max_output_tokens: super::TEST_MAX_OUTPUT_TOKENS,
            probe: true,
        };
        super::complete(&route, &request, &Cancel::new())
    })
    .await
    .map_err(|e| e.to_string())?
    .map(|_| format!("{} answered, using {model}.", kind.label()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("archgen-ai-settings-test-{}-{}", std::process::id(), uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // ── load / store ────────────────────────────────────────────────────────

    #[test]
    fn a_missing_settings_file_is_the_default_and_a_damaged_one_is_an_error() {
        let dir = TempDir::new();
        let path = dir.0.join("ai-settings.json");
        assert_eq!(load(&path).unwrap(), Settings::default());

        std::fs::write(&path, "{ this is not json").unwrap();
        let error = load(&path).unwrap_err();
        assert!(error.contains("damaged"), "{error}");
        // A damaged file is reported, never silently replaced by the default.
        assert!(load(&path).is_err());
    }

    #[test]
    fn store_writes_atomically_and_leaves_no_temporary_file_behind() {
        let dir = TempDir::new();
        let path = dir.0.join("nested/ai-settings.json");
        let mut settings = Settings::default();
        settings.timeout_seconds = 120;
        settings.provider = ProviderKind::Anthropic;
        store(&path, &settings).unwrap();
        assert_eq!(load(&path).unwrap(), settings);
        let siblings: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(siblings, vec![std::ffi::OsString::from("ai-settings.json")]);
    }

    /// Concurrent writers must never interleave a temp-file-plus-rename: every reader in the
    /// middle of the race sees either a missing file or one complete, parseable settings file.
    #[test]
    fn concurrent_stores_never_corrupt_the_file() {
        let dir = TempDir::new();
        let path = std::sync::Arc::new(dir.0.join("ai-settings.json"));
        let writers: Vec<_> = (0u64..8)
            .map(|n| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let mut settings = Settings::default();
                    settings.timeout_seconds = 30 + n;
                    for _ in 0..20 {
                        store(&path, &settings).unwrap();
                        assert!((30..38).contains(&load(&path).unwrap().timeout_seconds));
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        assert!(load(&path).is_ok());
    }

    /// Exercises `configure_at` — the exact body `ai_configure` runs — rather than a
    /// hand-composed `apply_configure(..).and_then(store)` sequence, so this proves the actual
    /// command never touches the file on a rejected call, not just that `Result::and_then`
    /// short-circuits (`rust-test-quality#10`).
    #[test]
    fn a_rejected_configure_never_writes_the_settings_file() {
        let dir = TempDir::new();
        let path = dir.0.join("ai-settings.json");
        let mut settings = Settings::default();
        settings.provider = ProviderKind::Anthropic;
        store(&path, &settings).unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(20));

        let result = configure_at(&path, None, Some("bad model!".into()), None, None);
        assert!(result.is_err());

        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before, "the file must be untouched");
        assert_eq!(load(&path).unwrap(), settings);
    }

    /// `ai_set_key`'s real body, run against a temp file: the key is never written to disk in
    /// the clear, and a hermetic re-read (injected env, so a real `OPENAI_API_KEY` on this
    /// machine cannot change the outcome) reports `keySource: "settings"` with the key absent
    /// from the serialised `AiStatus` — `keySource: "settings"` was never asserted by any status
    /// test before (`rust-test-quality#10`).
    #[test]
    fn set_key_at_stores_the_key_protected_and_never_leaks_it_to_disk_or_status() {
        let dir = TempDir::new();
        let path = dir.0.join("ai-settings.json");
        set_key_at(&path, "openai".into(), "sk-round-trip-test-key").unwrap();

        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(!on_disk.contains("sk-round-trip-test-key"));
        let settings = load(&path).unwrap();
        assert!(settings.keys.get("openai").unwrap().starts_with("dpapi:"));

        let status = status_with(&settings, |_| None, |_| None);
        let openai = status.providers.iter().find(|p| p.id == "openai").unwrap();
        assert_eq!(openai.key_source, "settings");
        let serialized = serde_json::to_string(&status).unwrap();
        assert!(!serialized.contains("sk-round-trip-test-key") && !serialized.contains("dpapi:"));
    }

    // ── local_ollama_url ────────────────────────────────────────────────────

    #[test]
    fn local_ollama_url_accepts_only_an_exact_loopback_origin() {
        for accepted in [
            "http://localhost",
            "https://localhost",
            "http://127.0.0.1",
            "http://[::1]",
            "http://localhost:11434",
            "http://localhost:11434/",
            "http://127.0.0.1:1",
            "http://localhost:65535",
            "http://[::1]:11434/",
        ] {
            assert!(local_ollama_url(accepted).is_ok(), "{accepted}");
        }
        assert_eq!(local_ollama_url("http://localhost:11434/").unwrap(), "http://localhost:11434");
        assert_eq!(local_ollama_url("http://[::1]/").unwrap(), "http://[::1]");
    }

    #[test]
    fn local_ollama_url_rejects_every_vector_the_contract_lists() {
        for rejected in [
            "http://localhost.evil.com",
            "http://localhost:11434@evil.com",
            "http://u:p@localhost",
            "http://127.0.0.1.nip.io",
            "http://127.1",
            "http://2130706433",
            "http://0.0.0.0",
            "http://[::ffff:127.0.0.1]",
            "http://localhost:0",
            "http://localhost:99999",
            "http://localhost:11434/api",
            "http://localhost?x",
            "http://localhost#x",
            "ftp://localhost",
            "http://localhost\n",
            "",
            "localhost",
            "http://",
        ] {
            assert!(local_ollama_url(rejected).is_err(), "{rejected:?} must be rejected");
        }
    }

    #[test]
    fn local_ollama_url_rejects_vectors_beyond_the_contracts_mandatory_list() {
        for rejected in [
            "HTTP://localhost",                // uppercase scheme
            "Http://localhost",                // mixed-case scheme
            "http://LOCALHOST",                // uppercase host
            "http://localhost.",               // trailing dot on the host
            "http://localhost:",               // colon with no port digits
            "http://localhost:+80",            // signed port
            "http://localhost: 80",            // space inside the port
            " http://localhost",               // leading space
            "http://localhost ",               // trailing space
            "http:// localhost",               // space right after the scheme
            "http://ⅼocalhost",                // Roman numeral U+217C look-alike for 'l'
            "http://lосalhost",                // Cyrillic 'о' (U+043E) look-alike
            "http://localhost%00.evil.com",    // embedded percent-encoded NUL, never decoded
            "http://localhost/%00",
        ] {
            assert!(local_ollama_url(rejected).is_err(), "{rejected:?} must be rejected");
        }
    }

    // ── route ────────────────────────────────────────────────────────────────

    #[test]
    fn route_requires_a_local_non_cloud_model_for_ollama() {
        assert_eq!(route(&Settings::default(), ProviderKind::Ollama).unwrap_err(), "Choose an Ollama model in AI settings.");

        let mut settings = Settings::default();
        settings.models.insert("ollama".into(), "qwen3.8:cloud".into());
        assert!(route(&settings, ProviderKind::Ollama).unwrap_err().contains("ollama.com"));

        settings.models.insert("ollama".into(), "qwen3.8:27b".into());
        let resolved = route(&settings, ProviderKind::Ollama).unwrap();
        assert_eq!(resolved.model, "qwen3.8:27b");
        assert!(resolved.api_key.is_none());
    }

    #[test]
    fn route_clamps_a_hand_edited_out_of_range_timeout() {
        let mut settings = Settings::default();
        settings.models.insert("ollama".into(), "qwen3.8:27b".into());

        settings.timeout_seconds = 1; // below the 30 s floor
        assert_eq!(route(&settings, ProviderKind::Ollama).unwrap().timeout, Duration::from_secs(30));

        settings.timeout_seconds = 999_999; // above the 3600 s ceiling
        assert_eq!(route(&settings, ProviderKind::Ollama).unwrap().timeout, Duration::from_secs(3600));
    }

    #[test]
    fn route_re_validates_a_hand_edited_lan_ollama_url() {
        let mut settings = Settings::default();
        settings.models.insert("ollama".into(), "qwen3.8:27b".into());
        settings.ollama_url = "http://192.168.1.5:11434".into();
        assert!(route(&settings, ProviderKind::Ollama).is_err(), "a LAN URL must never resolve");
    }

    /// No test needs a real `cli::locate`: `no_locate`/`located` stand in for "CLI absent" /
    /// "CLI found at some path", so a CLI-provider test's outcome never depends on what happens
    /// to be installed on the machine running the suite.
    fn no_locate(_: ProviderKind) -> Option<PathBuf> {
        None
    }
    fn located(_: ProviderKind) -> Option<PathBuf> {
        Some(PathBuf::from("stub-cli.exe"))
    }

    #[test]
    fn route_prefers_an_environment_key_and_names_a_malformed_one() {
        let mut settings = Settings::default();
        settings.provider = ProviderKind::Openai;

        let route1 = route_with(&settings, ProviderKind::Openai, &|name| (name == "OPENAI_API_KEY").then(|| " sk-good-key-12345 ".to_string()), &no_locate)
            .unwrap();
        assert_eq!(route1.api_key.as_deref(), Some("sk-good-key-12345"));
        assert_eq!(route1.model, "gpt-5.6");

        let error =
            route_with(&settings, ProviderKind::Openai, &|name| (name == "OPENAI_API_KEY").then(|| "has a space".to_string()), &no_locate).unwrap_err();
        assert_eq!(error, "OPENAI_API_KEY contains whitespace or control characters.");

        settings.models.insert("openai".into(), "gpt-4o".into());
        let route2 = route_with(&settings, ProviderKind::Openai, &|name| (name == "OPENAI_API_KEY").then(|| "sk-key".to_string()), &no_locate).unwrap();
        assert_eq!(route2.model, "gpt-4o");
    }

    #[test]
    fn route_reports_a_missing_key_with_a_sentence_naming_the_variable() {
        assert_eq!(
            route_with(&Settings::default(), ProviderKind::Openai, &|_| None, &no_locate).unwrap_err(),
            "Set an OpenAI API key in AI settings or start ArchGen with OPENAI_API_KEY."
        );
        assert_eq!(
            route_with(&Settings::default(), ProviderKind::Gemini, &|_| None, &no_locate).unwrap_err(),
            "Set a Gemini API key in AI settings or start ArchGen with GEMINI_API_KEY."
        );
    }

    #[test]
    fn route_falls_back_to_a_stored_key_and_a_corrupted_one_counts_as_none() {
        let mut settings = Settings::default();
        settings.keys.insert(ProviderKind::Anthropic.id().into(), super::super::secret::protect("sk-ant-stored").unwrap());
        let resolved = route_with(&settings, ProviderKind::Anthropic, &|_| None, &no_locate).unwrap();
        assert_eq!(resolved.api_key.as_deref(), Some("sk-ant-stored"));

        settings.keys.insert(ProviderKind::Anthropic.id().into(), "dpapi:not-really-protected-data".into());
        assert_eq!(
            route_with(&settings, ProviderKind::Anthropic, &|_| None, &no_locate).unwrap_err(),
            "Set an Anthropic API key in AI settings or start ArchGen with ANTHROPIC_API_KEY."
        );

        // A hand-edited file can put anything after "dpapi:", including non-ASCII text that
        // used to panic `secret::unprotect`'s byte-index slicing; route() and status() must
        // still just treat it as no key, never crash the caller.
        settings.keys.insert(ProviderKind::Anthropic.id().into(), "dpapi:€žžžžžžž🦄".into());
        assert_eq!(
            route_with(&settings, ProviderKind::Anthropic, &|_| None, &no_locate).unwrap_err(),
            "Set an Anthropic API key in AI settings or start ArchGen with ANTHROPIC_API_KEY."
        );
        let status = status_with(&settings, |_| None, no_locate);
        let anthropic = status.providers.iter().find(|p| p.id == "anthropic").unwrap();
        assert!(!anthropic.available && anthropic.key_source == "none");
    }

    #[test]
    fn route_rejects_a_model_starting_with_a_dash_for_every_kind_of_provider() {
        let mut settings = Settings::default();
        settings.models.insert("openai".into(), "-rf".into());
        let error = route_with(&settings, ProviderKind::Openai, &|name| (name == "OPENAI_API_KEY").then(|| "sk-key".to_string()), &no_locate).unwrap_err();
        assert_eq!(error, "\"-rf\" is not a valid model name.");

        settings.models.insert("ollama".into(), "-rf".into());
        assert_eq!(route(&settings, ProviderKind::Ollama).unwrap_err(), "Choose an Ollama model in AI settings.");

        settings.models.insert("claude_cli".into(), "-rf".into());
        let error = route_with(&settings, ProviderKind::ClaudeCli, &|_| None, &located).unwrap_err();
        assert_eq!(error, "\"-rf\" is not a valid model name.");
    }

    #[test]
    fn valid_key_and_valid_model_hold_their_documented_bounds() {
        assert!(valid_key("sk-1234567890"));
        assert!(valid_key(&"x".repeat(512)));
        assert!(!valid_key(""));
        assert!(!valid_key(&"x".repeat(513)));
        assert!(!valid_key("has space"));
        assert!(!valid_key("tab\tinside"));
        assert!(!valid_key("newline\ninside"));
        assert!(!valid_key("žluťoučký"));

        assert!(valid_model("qwen3.8:27b"));
        assert!(valid_model("org/model-name.v2"));
        assert!(!valid_model(""));
        assert!(!valid_model("-leading-dash"));
        assert!(!valid_model(&"x".repeat(121)));
        assert!(!valid_model("has space"));
        assert!(!valid_model("semi;colon"));
    }

    /// `contract-completeness#9`/`#10`: `route_with` used to call the real `cli::locate`
    /// directly, so its outcome — and therefore this test — depended on whether Claude/Codex/
    /// Gemini happened to be installed on the machine running the suite. Injecting `locate`
    /// makes both the "absent" and "present" branches, and their order relative to the model
    /// check, deterministic everywhere.
    #[test]
    fn route_fails_closed_for_every_cli_kind_when_locate_finds_nothing() {
        for kind in [ProviderKind::ClaudeCli, ProviderKind::CodexCli, ProviderKind::GeminiCli] {
            let error = route_with(&Settings::default(), kind, &|_| None, &no_locate).unwrap_err();
            assert_eq!(error, format!("{} is not installed or not on PATH.", kind.label()));
        }
    }

    /// Pins the order the code documents: `locate` runs before the model check, so a CLI that
    /// is not installed is reported even for a model that would otherwise also be rejected.
    #[test]
    fn route_checks_cli_locate_before_the_cli_models_name() {
        let mut settings = Settings::default();
        settings.models.insert("claude_cli".into(), "-rf".into());
        let error = route_with(&settings, ProviderKind::ClaudeCli, &|_| None, &no_locate).unwrap_err();
        assert_eq!(error, "Claude CLI is not installed or not on PATH.");
    }

    #[test]
    fn route_succeeds_for_a_located_cli_with_no_configured_model() {
        let resolved = route_with(&Settings::default(), ProviderKind::ClaudeCli, &|_| None, &located).unwrap();
        assert_eq!(resolved.kind, ProviderKind::ClaudeCli);
        assert!(resolved.model.is_empty());
    }

    /// `contract-completeness#5`: `provider_status` used to check only `cli::locate` for a CLI
    /// provider and only `find_key` for an API one, never the stored model — so an invalid
    /// stored model showed `available: true` in `status()` although `route()` would reject the
    /// same request. Deriving `provider_status` from `route_with` closes that gap for both.
    #[test]
    fn status_flags_an_invalid_stored_model_that_route_would_also_reject() {
        let mut settings = Settings::default();
        settings.models.insert("claude_cli".into(), "--dangerously-skip-permissions".into());
        let status = status_with(&settings, |_| None, &located);
        let claude_cli = status.providers.iter().find(|p| p.id == "claude_cli").unwrap();
        assert!(!claude_cli.available, "{claude_cli:?}");
        assert_eq!(claude_cli.detail, "\"--dangerously-skip-permissions\" is not a valid model name.");

        let mut settings = Settings::default();
        settings.models.insert("openai".into(), "gpt 4o".into());
        let status = status_with(&settings, |name| (name == "OPENAI_API_KEY").then(|| "sk-key".to_string()), &no_locate);
        let openai = status.providers.iter().find(|p| p.id == "openai").unwrap();
        assert!(!openai.available, "{openai:?}");
        assert_eq!(openai.detail, "\"gpt 4o\" is not a valid model name.");
        assert_eq!(openai.key_source, "env", "the key source must stay accurate even though the model is rejected");
    }

    // ── status ───────────────────────────────────────────────────────────────

    #[test]
    fn status_matches_the_fixtures_shape_and_provider_order_without_touching_the_environment() {
        let fixture: Value = serde_json::from_str(include_str!("../../../tests/fixtures/ai-status.json")).unwrap();
        let actual = serde_json::to_value(status(&Settings::default())).unwrap();

        fn sorted_keys(value: &Value) -> Vec<String> {
            let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        }
        assert_eq!(sorted_keys(&actual), sorted_keys(&fixture));

        let actual_providers = actual["providers"].as_array().unwrap();
        let fixture_providers = fixture["providers"].as_array().unwrap();
        assert_eq!(actual_providers.len(), 7);
        assert_eq!(actual_providers.len(), fixture_providers.len());
        for (kind, (actual_provider, fixture_provider)) in
            ProviderKind::ALL.iter().zip(actual_providers.iter().zip(fixture_providers))
        {
            assert_eq!(actual_provider["id"], serde_json::json!(kind.id()));
            assert_eq!(sorted_keys(actual_provider), sorted_keys(fixture_provider));
            let source = actual_provider["keySource"].as_str().unwrap();
            assert!(["env", "settings", "none", "not_needed"].contains(&source), "{source}");
        }
    }

    /// The shape test above only compares key sets, so a value drifting between `mod.rs` and
    /// the fixture (which `tests/ai.test.mjs` and the e2e tests also rely on) would be
    /// invisible (`rust-test-quality#10`). This pins every field by value, hermetically:
    /// injected settings/env/locate reproduce exactly the situation the fixture describes
    /// (a chosen Ollama model, an environment Anthropic key, Claude/Codex CLIs found, Gemini
    /// CLI not), with no dependency on this machine's real environment or PATH.
    #[test]
    fn status_matches_the_fixture_exactly_given_matching_settings_env_and_locate() {
        let fixture: Value = serde_json::from_str(include_str!("../../../tests/fixtures/ai-status.json")).unwrap();
        let mut settings = Settings::default();
        settings.models.insert("ollama".into(), "qwen3.8:27b".into());
        let env = |name: &str| (name == "ANTHROPIC_API_KEY").then(|| "sk-ant-fixture-key".to_string());
        let locate = |kind: ProviderKind| (kind != ProviderKind::GeminiCli).then(|| PathBuf::from("stub-cli.exe"));
        let actual = serde_json::to_value(status_with(&settings, env, locate)).unwrap();
        assert_eq!(actual, fixture);
    }

    #[test]
    fn status_never_fails_and_flags_invalid_stored_values_instead() {
        let mut settings = Settings::default();
        settings.ollama_url = "http://192.168.1.5".into();
        settings.models.insert("ollama".into(), "qwen3.8:27b".into());
        settings.timeout_seconds = 5; // below the clamp
        let status = status_with(&settings, |_| None, no_locate);
        let ollama = status.providers.iter().find(|p| p.id == "ollama").unwrap();
        assert!(!ollama.available && !ollama.detail.is_empty());
        assert_eq!(status.timeout_seconds, 30);

        let cloud = { let mut s = settings.clone(); s.ollama_url = DEFAULT_OLLAMA_URL.into(); s.models.insert("ollama".into(), "x:cloud".into()); s };
        let status = status_with(&cloud, |_| None, no_locate);
        let ollama = status.providers.iter().find(|p| p.id == "ollama").unwrap();
        assert!(!ollama.available && ollama.detail.contains("ollama.com"));
    }

    #[test]
    fn status_reports_where_a_cloud_providers_key_came_from() {
        let settings = { let mut s = Settings::default(); s.provider = ProviderKind::Anthropic; s };
        let via_env = status_with(&settings, |name| (name == "ANTHROPIC_API_KEY").then(|| "sk-ant-env-key".to_string()), no_locate);
        let anthropic = via_env.providers.iter().find(|p| p.id == "anthropic").unwrap();
        assert!(anthropic.available && anthropic.key_source == "env" && anthropic.detail.contains("ANTHROPIC_API_KEY"));

        let unavailable = status_with(&settings, |_| None, no_locate);
        let anthropic = unavailable.providers.iter().find(|p| p.id == "anthropic").unwrap();
        assert!(!anthropic.available && anthropic.key_source == "none");
    }

    // ── ai_configure / ai_set_key (pure bodies) ────────────────────────────────

    #[test]
    fn apply_configure_validates_everything_before_writing_anything() {
        let mut settings = Settings::default();
        apply_configure(&mut settings, Some("openai".into()), Some(" gpt-4o ".into()), None, Some(10)).unwrap();
        assert_eq!(settings.provider, ProviderKind::Openai);
        assert_eq!(settings.models.get("openai"), Some(&"gpt-4o".to_string()));
        assert_eq!(settings.timeout_seconds, 30); // clamped up from 10

        apply_configure(&mut settings, None, Some(String::new()), None, None).unwrap();
        assert!(!settings.models.contains_key("openai"));

        let before = settings.clone();
        assert!(apply_configure(&mut settings, None, Some("bad model!".into()), None, None).is_err());
        assert!(apply_configure(&mut settings, Some("nope".into()), None, None, None).is_err());
        assert!(apply_configure(&mut settings, None, None, Some("http://192.168.1.1".into()), None).is_err());
        assert_eq!(settings, before, "a rejected configure must not partially apply");

        assert!(apply_configure(&mut settings, Some("ollama".into()), Some("qwen3.8:cloud".into()), None, None).is_err());

        settings.ollama_url = "http://localhost:9999".into();
        apply_configure(&mut settings, None, None, Some(String::new()), None).unwrap();
        assert_eq!(settings.ollama_url, DEFAULT_OLLAMA_URL);
    }

    #[test]
    fn apply_set_key_rejects_keyless_providers_and_protects_a_stored_key() {
        let mut settings = Settings::default();
        assert!(apply_set_key(&mut settings, ProviderKind::Ollama, "irrelevant").is_err());
        assert!(apply_set_key(&mut settings, ProviderKind::Openai, "has a space").is_err());

        apply_set_key(&mut settings, ProviderKind::Openai, "sk-good-key").unwrap();
        let stored = settings.keys.get("openai").unwrap().clone();
        assert!(stored.starts_with("dpapi:") && stored != "sk-good-key");
        assert_eq!(super::super::secret::unprotect(&stored).as_deref(), Some("sk-good-key"));

        apply_set_key(&mut settings, ProviderKind::Openai, "").unwrap();
        assert!(!settings.keys.contains_key("openai"));
    }
}
