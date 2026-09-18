//! One way to ask any assistant for a JSON answer: local Ollama, a paid API
//! (OpenAI, Gemini, Anthropic) or an installed agent CLI (Claude, Codex, Gemini).
//!
//! Every provider gets a system prompt, one user message and a JSON Schema, and
//! must hand back a value that satisfies the schema. Providers never write to the
//! project: CLIs run read-only in an empty temporary folder with tools disabled,
//! and the answer only ever becomes a proposal the user reviews (see `runtime.rs`).
//!
//! Nothing here needs a new crate: HTTP goes through the system `curl.exe`, with
//! the API key on curl's stdin config, never on a command line or in a file.
mod cli;
mod http;
pub mod process;
pub mod rework;
pub mod schema;
mod secret;
pub mod settings;

pub use process::Cancel;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Ollama,
    Openai,
    Gemini,
    Anthropic,
    ClaudeCli,
    CodexCli,
    GeminiCli,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 7] = [
        ProviderKind::Ollama,
        ProviderKind::Openai,
        ProviderKind::Gemini,
        ProviderKind::Anthropic,
        ProviderKind::ClaudeCli,
        ProviderKind::CodexCli,
        ProviderKind::GeminiCli,
    ];

    /// The identifier used in settings, jobs and IPC. Same names as Own IDE.
    pub fn id(self) -> &'static str {
        match self {
            ProviderKind::Ollama => "ollama",
            ProviderKind::Openai => "openai",
            ProviderKind::Gemini => "gemini",
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::ClaudeCli => "claude_cli",
            ProviderKind::CodexCli => "codex_cli",
            ProviderKind::GeminiCli => "gemini_cli",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Ollama => "Ollama (local)",
            ProviderKind::Openai => "OpenAI API",
            ProviderKind::Gemini => "Gemini API",
            ProviderKind::Anthropic => "Claude API",
            ProviderKind::ClaudeCli => "Claude CLI",
            ProviderKind::CodexCli => "Codex CLI",
            ProviderKind::GeminiCli => "Gemini CLI",
        }
    }

    /// Only a local provider keeps the selected content on this computer.
    pub fn is_local(self) -> bool {
        self == ProviderKind::Ollama
    }

    pub fn is_cli(self) -> bool {
        matches!(
            self,
            ProviderKind::ClaudeCli | ProviderKind::CodexCli | ProviderKind::GeminiCli
        )
    }

    pub fn needs_key(self) -> bool {
        matches!(
            self,
            ProviderKind::Openai | ProviderKind::Gemini | ProviderKind::Anthropic
        )
    }

    /// Who receives the content, for the consent text next to the Generate button.
    pub fn recipient(self) -> &'static str {
        match self {
            ProviderKind::Ollama => "this computer",
            ProviderKind::Openai | ProviderKind::CodexCli => "OpenAI",
            ProviderKind::Gemini | ProviderKind::GeminiCli => "Google",
            ProviderKind::Anthropic | ProviderKind::ClaudeCli => "Anthropic",
        }
    }

    /// Empty means "no default": Ollama needs an installed model chosen by the
    /// user, and a CLI without a model uses the one it is configured with.
    pub fn default_model(self) -> &'static str {
        match self {
            ProviderKind::Openai => "gpt-5.6",
            ProviderKind::Gemini => "gemini-3.8-flash",
            ProviderKind::Anthropic => "claude-opus-5",
            _ => "",
        }
    }

    /// An extra sentence shown with the consent text, for anything the user must know
    /// beyond "who receives the content". Empty when there is nothing to add.
    pub fn notice(self) -> &'static str {
        ""
    }

    /// Environment variables that may carry the API key, in priority order.
    pub fn key_variables(self) -> &'static [&'static str] {
        match self {
            ProviderKind::Openai => &["OPENAI_API_KEY"],
            ProviderKind::Gemini => &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
            ProviderKind::Anthropic => &["ANTHROPIC_API_KEY"],
            _ => &[],
        }
    }
}

/// Everything one request needs to reach its provider. Built by `Ai::route`;
/// the key lives only here and in the child process's stdin.
pub struct Route {
    pub kind: ProviderKind,
    /// Validated by `settings::valid_model`; may be empty for a CLI.
    pub model: String,
    pub api_key: Option<String>,
    /// Loopback origin such as `http://localhost:11434`, already validated.
    pub ollama_url: String,
    pub timeout: Duration,
}

impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Route")
            .field("kind", &self.kind)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "[redacted]"))
            .field("ollama_url", &self.ollama_url)
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// The answer budget of a connectivity probe (`ai_test_provider`, the `live_*` tests): far above
/// the one-word answer, because current models spend part of the cap on hidden reasoning.
pub const TEST_MAX_OUTPUT_TOKENS: u32 = 4096;

pub struct Completion<'a> {
    pub system: &'a str,
    pub user: &'a str,
    /// Strict JSON Schema: every object lists all its properties as required and
    /// sets `additionalProperties: false`, so OpenAI strict mode accepts it as is.
    pub schema: &'a Value,
    pub max_output_tokens: u32,
    /// A connectivity probe rather than real work: providers keep hidden reasoning minimal
    /// (Ollama `think: false`, Anthropic low effort). Never set for a rework request.
    pub probe: bool,
}

/// The single entry point. Whatever the provider enforced, the answer is
/// checked against the schema again here before anyone else sees it.
pub fn complete(route: &Route, request: &Completion, cancel: &Cancel) -> Result<Value, String> {
    if cancel.is_cancelled() {
        return Err(process::CANCELLED.into());
    }
    let answer = if route.kind.is_cli() {
        cli::complete(route, request, cancel)
    } else {
        http::complete(route, request, cancel)
    };
    answer
        .and_then(|value| {
            schema::check(&value, request.schema)
                .map_err(|e| format!("The assistant's answer did not have the expected shape: {e}"))?;
            Ok(value)
        })
        .map_err(|error| redact(&error, route))
}

/// Errors end up in the job registry and on screen, and they may quote what a provider,
/// a proxy or curl printed. No key of any provider survives this, and no error is long.
fn redact(error: &str, route: &Route) -> String {
    if error == process::CANCELLED {
        return error.into();
    }
    let mut secrets: Vec<String> = route.api_key.iter().cloned().collect();
    secrets.extend(
        ProviderKind::ALL
            .iter()
            .flat_map(|kind| kind.key_variables())
            .filter_map(|name| std::env::var(name).ok()),
    );
    let mut text = error.to_string();
    for secret in secrets.iter().map(|s| s.trim()).filter(|s| s.len() >= 8) {
        text = text.replace(secret, "[redacted]");
    }
    process::truncate(&text, 600)
}

/// Tauri-managed state: where the settings live and which jobs can be stopped.
pub struct Ai {
    settings_path: PathBuf,
    running: Mutex<HashMap<String, Cancel>>,
}

impl Ai {
    pub fn new(app_data: &Path) -> Self {
        // Longer than any request may run (the timeout is capped at one hour).
        process::sweep_stale_temp_dirs(Duration::from_secs(2 * 3600));
        // Gemini CLI keeps every prompt on disk; a request that died with ArchGen never cleaned up.
        cli::sweep_stale_gemini_sessions();
        Self {
            settings_path: app_data.join("ai-settings.json"),
            running: Mutex::new(HashMap::new()),
        }
    }

    /// Same as `new`, without the `%TEMP%` sweep: `new` deletes every stale
    /// `archgen-ai-*` folder under the *real* system temp directory as a side effect, which a
    /// test must never do to the developer's actual `%TEMP%` (`rust-test-quality#10`). Tests
    /// that only need an `Ai` value — never a live provider — construct it this way instead.
    #[cfg(test)]
    pub(crate) fn without_sweep(app_data: &Path) -> Self {
        Self {
            settings_path: app_data.join("ai-settings.json"),
            running: Mutex::new(HashMap::new()),
        }
    }

    pub fn settings_path(&self) -> &Path {
        &self.settings_path
    }

    /// The route for the configured provider, or an actionable error
    /// (no key, no Ollama model chosen, CLI not installed).
    pub fn route(&self) -> Result<Route, String> {
        let settings = settings::load(&self.settings_path)?;
        settings::route(&settings, settings.provider)
    }

    pub fn route_for(&self, kind: ProviderKind) -> Result<Route, String> {
        settings::route(&settings::load(&self.settings_path)?, kind)
    }

    /// Register a running job so `cancel` can stop its child process.
    pub fn begin(&self, job_id: &str) -> Cancel {
        let cancel = Cancel::new();
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(job_id.into(), cancel.clone());
        cancel
    }

    pub fn end(&self, job_id: &str) {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(job_id);
    }

    /// On exit: every request stops, so no provider keeps uploading after the window is gone.
    pub fn cancel_all(&self) {
        for cancel in self.running.lock().unwrap_or_else(|e| e.into_inner()).values() {
            cancel.cancel();
        }
        // A cancelled request still has to remove what its provider wrote (Gemini's session
        // files); give it a moment before the process goes away. The start-up sweeps cover the rest.
        for _ in 0..60 {
            if self.running.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Stops the provider process of a job, if it has one. Safe to call for any job.
    pub fn cancel(&self, job_id: &str) {
        let cancel = self
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(job_id)
            .cloned();
        if let Some(cancel) = cancel {
            cancel.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_ids_roundtrip_and_match_serde() {
        for kind in ProviderKind::ALL {
            assert_eq!(ProviderKind::parse(kind.id()), Some(kind));
            assert_eq!(serde_json::to_value(kind).unwrap(), serde_json::json!(kind.id()));
        }
        assert_eq!(ProviderKind::parse("OLLAMA"), None);
        assert!(ProviderKind::Ollama.is_local() && !ProviderKind::ClaudeCli.is_local());
    }

    #[test]
    fn route_debug_never_prints_the_key() {
        let route = Route {
            kind: ProviderKind::Openai,
            model: "m".into(),
            api_key: Some("sk-secret".into()),
            ollama_url: String::new(),
            timeout: Duration::from_secs(1),
        };
        assert!(!format!("{route:?}").contains("sk-secret"));
    }

    #[test]
    fn errors_never_carry_a_key_and_stay_short() {
        let route = Route {
            kind: ProviderKind::Openai,
            model: "m".into(),
            api_key: Some("sk-live-1234567890".into()),
            ollama_url: String::new(),
            timeout: Duration::from_secs(1),
        };
        let echoed = format!("curl: option 'sk-live-1234567890\"' is unknown {}", "x".repeat(2000));
        let shown = redact(&echoed, &route);
        assert!(!shown.contains("sk-live") && shown.contains("[redacted]"));
        assert!(shown.chars().count() <= 601);
        assert_eq!(redact(process::CANCELLED, &route), process::CANCELLED);
    }

    #[test]
    fn a_cancelled_request_never_reaches_a_provider() {
        let ai = Ai::without_sweep(Path::new("unused"));
        let cancel = ai.begin("job");
        ai.cancel("job");
        ai.cancel("unknown job");
        assert!(cancel.is_cancelled());
        ai.end("job");
        // Exit waits for running requests to clean up, but only until they are done.
        let other = ai.begin("other");
        let started = std::time::Instant::now();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                while !other.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                ai.end("other");
            });
            ai.cancel_all();
        });
        assert!(other.is_cancelled());
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        let route = Route {
            kind: ProviderKind::ClaudeCli,
            model: String::new(),
            api_key: None,
            ollama_url: String::new(),
            timeout: Duration::from_secs(1),
        };
        let schema = serde_json::json!({"type": "object"});
        let request = Completion { system: "", user: "", schema: &schema, max_output_tokens: 1, probe: false };
        assert_eq!(complete(&route, &request, &cancel).unwrap_err(), process::CANCELLED);
        ai.end("job");
    }
}
