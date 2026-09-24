//! Agent CLIs as providers: Claude Code, Codex and Gemini in headless mode.
//!
//! Every CLI is invoked the same way: locate its executable (never starting a
//! process to do so), probe `--help` once to learn which safety flags this
//! install actually supports (a provider whose help omits one fails closed
//! with one sentence, per the design contract), build an argument list from
//! only documented flags, run it through `process::run` in an empty working
//! directory with its own tools disabled, and turn its answer into JSON.
//! `--ephemeral`/`--no-session-persistence` are the only flags added merely
//! when documented; everything else the contract calls "required" is checked
//! with `require_flag` and turns a missing flag into a refusal, never a
//! silently weaker invocation.
use super::{
    process::{self, Cancel},
    schema, Completion, ProviderKind, Route,
};
use serde_json::Value;
use std::{
    ffi::OsStr,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
    time::Duration,
};

// ── Help probe ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct CliHelp {
    path: PathBuf,
    text: String,
}

static CLAUDE_HELP: OnceLock<Result<CliHelp, String>> = OnceLock::new();
static CODEX_HELP: OnceLock<Result<CliHelp, String>> = OnceLock::new();
static GEMINI_HELP: OnceLock<Result<CliHelp, String>> = OnceLock::new();

fn cli_help(kind: ProviderKind) -> Result<CliHelp, String> {
    let (cache, args): (&'static OnceLock<Result<CliHelp, String>>, &[&str]) = match kind {
        ProviderKind::ClaudeCli => (&CLAUDE_HELP, &["--help"]),
        ProviderKind::CodexCli => (&CODEX_HELP, &["exec", "--help"]),
        ProviderKind::GeminiCli => (&GEMINI_HELP, &["--help"]),
        _ => return Err("not a CLI provider".into()),
    };
    cached_or_probe(cache, || probe_help(kind, args))
}

/// Cached only on success, so a CLI installed or signed in after ArchGen
/// started is picked up on the very next request instead of staying refused
/// for the rest of the session. Shared with the Codex `features list` probe.
fn cached_or_probe<T: Clone>(cache: &OnceLock<Result<T, String>>, probe: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    if let Some(cached) = cache.get() {
        return cached.clone();
    }
    let probed = probe();
    if probed.is_ok() {
        let _ = cache.set(probed.clone());
    }
    probed
}

/// Builds the `Command` a probe runs, with `dir` (an empty, private folder)
/// as its working directory — never ArchGen's own cwd, which an npm `.cmd`
/// shim's fallback branch would search for a bare `node` before PATH. `dir`
/// is also the child's `TEMP`/`TMP`: on this path only `--help`/`features
/// list` run, which make no API call and so should never invoke Gemini's
/// `reportError`, but a probe is still a Gemini/Codex child process, and
/// `gemini_command` below shows exactly what inheriting ArchGen's real
/// `%TEMP%` can leak — cheap insurance against a future probe argument that
/// does reach the network.
fn probe_command(path: &Path, args: &[&str], dir: &Path) -> Command {
    let mut cmd = Command::new(path);
    cmd.args(args).current_dir(dir).env("TEMP", dir).env("TMP", dir);
    cmd
}

fn run_probe(path: &Path, args: &[&str], timeout: Duration) -> Result<process::Output, String> {
    let dir = process::TempDir::new()?;
    process::run(probe_command(path, args, dir.path()), "", &process::Limits::new(timeout), &Cancel::new())
}

fn probe_help(kind: ProviderKind, args: &[&str]) -> Result<CliHelp, String> {
    let path = locate(kind).ok_or_else(|| format!("{} is not installed or not on PATH.", kind.label()))?;
    let output = run_probe(&path, args, Duration::from_secs(10))?;
    if !output.success {
        return Err(process::truncate(&output.stderr, 300));
    }
    Ok(CliHelp { path, text: output.stdout })
}

/// A line that documents `flag` as one of its own options, not merely
/// mentioning it in a longer description (own-ide's `documented_flag`). Real
/// option rows from commander/clap/yargs start at column 2 or 6; a wrapped
/// description line that happens to start with a dash-led word (Claude's
/// `--restricted` mentions `--tools`, its `--system-prompt-snapshot`
/// mentions `--system-prompt`) is indented to column 40 or more, so an
/// indent past 6 columns disqualifies a line from being an option row.
fn documented_flag(help: &str, flag: &str) -> bool {
    help.lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with('-') && line.len() - trimmed.len() <= 6
        })
        .any(|line| {
            line.split_whitespace()
                .take_while(|word| word.starts_with('-') || *word == ",")
                .any(|word| word.trim_end_matches(',') == flag)
        })
}

fn require_flag(help: &str, flag: &str, provider: &str) -> Result<(), String> {
    if documented_flag(help, flag) {
        Ok(())
    } else {
        Err(format!(
            "{provider} CLI on this computer is too old for ArchGen: it does not support {flag}."
        ))
    }
}

// ── Locate (no process started) ─────────────────────────────────────────────

pub fn locate(kind: ProviderKind) -> Option<PathBuf> {
    match kind {
        ProviderKind::ClaudeCli => locate_claude(
            std::env::var_os("PATH").as_deref(),
            std::env::var_os("USERPROFILE").as_deref(),
        ),
        ProviderKind::CodexCli => process::locate("codex"),
        ProviderKind::GeminiCli => process::locate("gemini"),
        _ => None,
    }
}

/// A native `claude.exe` needs no CR/LF folding and starts faster than the
/// npm shim; it wins wherever on `path` it is found, even over a `.cmd` shim
/// listed earlier. `path` is `PATH`, passed in rather than read here so tests
/// can hand it a constructed value.
fn locate_claude_exe(path: Option<&OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path?)
        .map(|dir| dir.join("claude.exe"))
        .find(|candidate| candidate.is_file())
}

fn locate_claude(path: Option<&OsStr>, user_profile: Option<&OsStr>) -> Option<PathBuf> {
    locate_claude_exe(path)
        .or_else(|| {
            let candidate = PathBuf::from(user_profile?).join(".local").join("bin").join("claude.exe");
            candidate.is_file().then_some(candidate)
        })
        .or_else(|| process::locate("claude"))
}

fn is_cmd_shim(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
}

// ── Claude Code ──────────────────────────────────────────────────────────────

/// `.cmd`/`.bat` targets go through `cmd.exe`, whose argument quoting cannot
/// carry an embedded CR/LF; Rust's `Command` refuses such an argument outright
/// rather than risk it breaking out of quoting. The native `.exe` has no such
/// restriction, so this only runs for a shim.
fn fold_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace(['\r', '\n'], " ")
}

/// The argv `complete_claude` runs. `--tools` is variadic, so its `""` value
/// must be followed by another flag, never a positional — every flag after it
/// here satisfies that.
fn claude_args(help: &str, model: &str, system: &str, schema: &Value, is_shim: bool) -> Result<Vec<String>, String> {
    for flag in ["-p", "--output-format", "--json-schema", "--system-prompt", "--tools", "--strict-mcp-config"] {
        require_flag(help, flag, "Claude")?;
    }
    let system = if is_shim { fold_crlf(system) } else { system.to_string() };
    let mut args = vec![
        "-p".to_string(),
        "--output-format".to_string(),
        "json".to_string(),
        "--json-schema".to_string(),
        schema.to_string(),
        "--system-prompt".to_string(),
        system,
        "--tools".to_string(),
        String::new(),
        "--strict-mcp-config".to_string(),
    ];
    if documented_flag(help, "--no-session-persistence") {
        args.push("--no-session-persistence".to_string());
    }
    if !model.is_empty() {
        require_flag(help, "--model", "Claude")?;
        args.push("--model".to_string());
        args.push(model.to_string());
    }
    Ok(args)
}

/// `claude -p`'s stdout, turned into the JSON value it carries. `is_error` is
/// checked first: a failed run still answers with a `result` string, but that
/// string is the failure reason, never an answer. `structured_output` is
/// preferred; a plain `result` is scanned for the first JSON object in it,
/// since a model can wrap its answer in a sentence despite the instructions.
fn parse_claude_output(stdout: &str) -> Result<Value, String> {
    let value: Value =
        serde_json::from_str(stdout.trim()).map_err(|e| format!("Claude Code's output was not JSON: {e}"))?;
    if value.get("is_error").and_then(Value::as_bool) == Some(true) {
        return Err(claude_error_reason(&value));
    }
    if let Some(structured) = value.get("structured_output").filter(|v| !v.is_null()) {
        return Ok(structured.clone());
    }
    match value.get("result").and_then(Value::as_str) {
        Some(text) if !text.trim().is_empty() => schema::first_json_object(text),
        _ => Err("Claude Code's reply had neither a structured answer nor a result.".into()),
    }
}

/// The failure reason from an `is_error:true` reply. The `success` subtype
/// puts it in `result`; the `error_*` subtypes (retry exhaustion, max turns,
/// max budget, an execution error) have no `result` field at all and put it
/// in `errors` instead — own-ide never saw those subtypes, so this contract
/// gap was never noticed there.
fn claude_error_reason(value: &Value) -> String {
    let result = value.get("result").and_then(Value::as_str).filter(|s| !s.trim().is_empty());
    let first_error = value
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|errors| errors.iter().find_map(Value::as_str))
        .filter(|s| !s.trim().is_empty());
    let subtype = value.get("subtype").and_then(Value::as_str);
    let reason = result
        .or(first_error)
        .map(str::to_string)
        .or_else(|| subtype.map(|s| format!("Claude Code stopped: {s}")))
        .unwrap_or_else(|| "Claude Code reported an error.".to_string());
    process::truncate(&reason, 300)
}

/// A non-zero exit whose stdout never became the one JSON object `claude -p`
/// promises means the real reason is on stderr instead (a crash, a flag an
/// older CLI rejects); an `is_error` reply, by contrast, *is* valid JSON and
/// is handled by `parse_claude_output`, not here.
fn claude_answer(output: &process::Output) -> Result<Value, String> {
    if !output.success && serde_json::from_str::<Value>(output.stdout.trim()).is_err() {
        return Err(if output.stderr.trim().is_empty() {
            "Claude Code exited without an answer.".to_string()
        } else {
            process::truncate(&output.stderr, 300)
        });
    }
    parse_claude_output(&output.stdout)
}

/// Same reasoning as `http::CLOUD_REASONING_ALLOWANCE`: `max_output_tokens` stays "how long the answer may be".
const CLAUDE_REASONING_ALLOWANCE: u32 = 8_000;

fn complete_claude(route: &Route, request: &Completion, cancel: &Cancel) -> Result<Value, String> {
    let help = cli_help(ProviderKind::ClaudeCli)?;
    let args = claude_args(&help.text, &route.model, request.system, request.schema, is_cmd_shim(&help.path))?;
    let cwd = process::TempDir::new()?;
    let mut cmd = Command::new(&help.path);
    cmd.args(args).current_dir(cwd.path());
    // `claude -p` has no flag for its output-token budget; it reads this environment variable
    // instead. The cap covers hidden reasoning as well as the answer, hence the allowance.
    cmd.env("CLAUDE_CODE_MAX_OUTPUT_TOKENS", (request.max_output_tokens + CLAUDE_REASONING_ALLOWANCE).to_string());
    let limits = process::Limits::new(route.timeout);
    let output = process::run(cmd, request.user, &limits, cancel)?;
    claude_answer(&output)
}

// ── Codex CLI ────────────────────────────────────────────────────────────────

/// Every Codex feature that grants a tool the model could call, if this
/// install has it: a shell, a file read via the shell/exec tools, a browser,
/// image generation, a plugin or another agent — whether or not this build
/// defaults it to enabled. `-s read-only` alone still lets the shell tool
/// read any file, so these are disabled on top of it, not instead of it. Web
/// search is not in this list: it is the separate `web_search` config key
/// (below), not a feature. `--disable` **rejects** a name it does not
/// recognize (exits non-zero before anything runs), unlike `-c
/// features.<name>=false`, which is lenient — so every name here is checked
/// against a live `codex features list` before use (`codex_features`).
const TOOL_LIKE_FEATURES: &[&str] = &[
    "shell_tool",
    "unified_exec",
    "unified_exec_tty",
    "view_image",
    "apps",
    "plugins",
    "plugin_sharing",
    "remote_plugin",
    "multi_agent",
    "multi_agent_v2",
    "browser_use",
    "browser_use_external",
    "browser_use_full_cdp_access",
    "computer_use",
    "code_mode_host",
    "image_generation",
    "sleep_tool",
    "skill_mcp_dependency_install",
    "skill_search",
    "request_permissions_tool",
    "in_app_local_automation",
    "tool_call_mcp_elicitation",
    "network_proxy",
];

static CODEX_FEATURES: OnceLock<Result<Vec<String>, String>> = OnceLock::new();

/// The feature names this Codex install actually has, from `codex features
/// list` (the first whitespace-delimited token of each stdout line; the
/// remaining columns are the stage and an enabled-by-default flag, neither
/// relevant here). Free and offline, but not usable with
/// `--ignore-user-config`, so it loads the user's own `config.toml`; cached
/// only on success, like the help probe.
fn codex_features(path: &Path) -> Result<Vec<String>, String> {
    cached_or_probe(&CODEX_FEATURES, || {
        let output = run_probe(path, &["features", "list"], Duration::from_secs(10))?;
        if !output.success {
            return Err(process::truncate(&output.stderr, 300));
        }
        Ok(parse_feature_names(&output.stdout))
    })
}

fn parse_feature_names(stdout: &str) -> Vec<String> {
    stdout.lines().filter_map(|line| line.split_whitespace().next().map(str::to_string)).collect()
}

/// The argv `complete_codex` runs. `known_features` is whatever
/// `codex_features` found on this install (a parameter so tests can inject a
/// shorter list, as an older Codex would report); the caller fails closed
/// before ever reaching here when the probe itself fails.
fn codex_args(
    help: &str,
    model: &str,
    schema: &Path,
    output: &Path,
    cwd: &Path,
    known_features: &[String],
) -> Result<Vec<String>, String> {
    for flag in ["--output-schema", "-o", "-s", "--skip-git-repo-check", "-C", "--ignore-user-config", "--disable", "-c"] {
        require_flag(help, flag, "Codex")?;
    }
    if !help.contains("read-only") {
        return Err("Codex CLI on this computer is too old for ArchGen: it does not support a read-only sandbox.".into());
    }
    let has = |name: &str| known_features.iter().any(|f| f == name);
    if !has("shell_tool") || !has("unified_exec") {
        return Err("Codex CLI on this computer cannot switch its tools off for ArchGen.".into());
    }
    let mut args = vec![
        "exec".to_string(),
        "--output-schema".to_string(),
        schema.to_string_lossy().into_owned(),
        "-o".to_string(),
        output.to_string_lossy().into_owned(),
        "-s".to_string(),
        "read-only".to_string(),
        "--skip-git-repo-check".to_string(),
        "-C".to_string(),
        cwd.to_string_lossy().into_owned(),
        "--ignore-user-config".to_string(),
        "-c".to_string(),
        "web_search=disabled".to_string(),
    ];
    for feature in TOOL_LIKE_FEATURES.iter().filter(|f| has(f)) {
        args.push("--disable".to_string());
        args.push((*feature).to_string());
    }
    if documented_flag(help, "--ephemeral") {
        args.push("--ephemeral".to_string());
    }
    if documented_flag(help, "--json") {
        args.push("--json".to_string());
    }
    if !model.is_empty() {
        require_flag(help, "--model", "Codex")?;
        args.push("--model".to_string());
        args.push(model.to_string());
    }
    args.push("-".to_string());
    Ok(args)
}

/// The last `error.message` of a `{"type":"turn.failed"}` event in `--json`'s
/// event stream, or else the last `message` of a `{"type":"error"}` event (a
/// dead connection can retry through several of those before the run ends).
/// `None` when `--json` was not used, or produced no event of either kind.
fn codex_json_failure(stdout: &str) -> Option<String> {
    fn event_type(event: &Value) -> Option<&str> {
        event.get("type").and_then(Value::as_str)
    }
    let events: Vec<Value> = stdout.lines().filter_map(|line| serde_json::from_str(line).ok()).collect();
    events
        .iter()
        .rev()
        .find(|e| event_type(e) == Some("turn.failed"))
        .and_then(|e| e.get("error").and_then(|err| err.get("message")).and_then(Value::as_str))
        .or_else(|| {
            events
                .iter()
                .rev()
                .find(|e| event_type(e) == Some("error"))
                .and_then(|e| e.get("message").and_then(Value::as_str))
        })
        .map(str::to_string)
}

/// Codex's stderr, on a non-zero exit, opens with a startup banner (workdir,
/// model, session id) and then the *entire* stdin prompt echoed back before
/// any `ERROR:` line — quoting its head would leak the document content the
/// contract says an error must never carry, and with ArchGen's temp-folder
/// workdir the banner alone is already close to the 300-character quote
/// limit. So this never reads from the front: it prefers a `--json` event
/// (see `codex_json_failure`), and otherwise scans stderr from the end for
/// the last line that actually reports an error.
fn codex_failure(stdout: &str, stderr: &str) -> String {
    if let Some(reason) = codex_json_failure(stdout) {
        return process::truncate(&reason, 300);
    }
    let is_error_line = |line: &&str| {
        let trimmed = line.trim_start();
        trimmed.starts_with("ERROR:") || trimmed.starts_with("Error:") || trimmed.starts_with("error:")
    };
    match stderr.lines().rev().find(is_error_line) {
        Some(line) => process::truncate(line, 300),
        None => "Codex CLI exited without an answer.".to_string(),
    }
}

/// The `-o` file's content: the bare JSON object Codex promises, no envelope.
fn codex_answer(bytes: &[u8]) -> Result<Value, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "Codex CLI's answer is not UTF-8 text.".to_string())?;
    serde_json::from_str(text.trim()).map_err(|e| format!("Codex CLI's answer was not JSON: {e}"))
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("reading Codex CLI's answer: {e}"))?;
    let mut buf = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("reading Codex CLI's answer: {e}"))?;
    if buf.len() > limit {
        return Err("Codex CLI's answer was larger than ArchGen accepts.".into());
    }
    Ok(buf)
}

fn complete_codex(route: &Route, request: &Completion, cancel: &Cancel) -> Result<Value, String> {
    let help = cli_help(ProviderKind::CodexCli)?;
    let known_features = codex_features(&help.path)
        .map_err(|_| "Codex CLI on this computer cannot switch its tools off for ArchGen.".to_string())?;
    let cwd = process::TempDir::new()?;
    let schema_path = cwd.write("schema.json", &request.schema.to_string())?;
    let output_path = cwd.write("output.json", "")?;
    let args = codex_args(&help.text, &route.model, &schema_path, &output_path, cwd.path(), &known_features)?;
    let mut cmd = Command::new(&help.path);
    cmd.args(args).current_dir(cwd.path());
    let limits = process::Limits::new(route.timeout);
    let stdin = format!("{}\n\n{}", request.system, request.user);
    let output = process::run(cmd, &stdin, &limits, cancel)?;
    if !output.success {
        return Err(codex_failure(&output.stdout, &output.stderr));
    }
    codex_answer(&read_bounded(&output_path, 8 * 1024 * 1024)?)
}

// ── Gemini CLI ───────────────────────────────────────────────────────────────

/// `--policy` files load at the *user* tier, one step below admin and above
/// every built-in default-tier policy (`plan.toml`, `read-only.toml`, …), so
/// this one rule beats all of them regardless of priority number — verified
/// against the syntax in the installed
/// `@google/gemini-cli/bundle/policies/*.toml` files.
const DENY_ALL_POLICY: &str = "[[rule]]\ntoolName = \"*\"\ndecision = \"deny\"\npriority = 999\n";

/// The argv `complete_gemini` runs; `-o json` is mandatory, not "use when
/// documented", because the parser below depends on the envelope it produces.
fn gemini_args(help: &str, model: &str, policy: &Path) -> Result<Vec<String>, String> {
    for flag in ["--prompt", "--approval-mode", "--skip-trust", "--policy", "--allowed-mcp-server-names", "-o"] {
        require_flag(help, flag, "Gemini")?;
    }
    if !help.contains("stdin") {
        return Err(
            "Gemini CLI on this computer is too old for ArchGen: it does not support appending the prompt to stdin.".into(),
        );
    }
    if !help.contains("plan") {
        return Err("Gemini CLI on this computer is too old for ArchGen: it does not support --approval-mode plan.".into());
    }
    if !help.contains("\"json\"") {
        return Err("Gemini CLI on this computer is too old for ArchGen: it does not support --output-format json.".into());
    }
    let mut args = vec![
        "--prompt".to_string(),
        "Follow the task supplied on stdin. Do not use tools.".to_string(),
        "--approval-mode".to_string(),
        "plan".to_string(),
        "--skip-trust".to_string(),
        "--policy".to_string(),
        policy.to_string_lossy().into_owned(),
        "--allowed-mcp-server-names".to_string(),
        "archgen-none".to_string(),
        "-o".to_string(),
        "json".to_string(),
    ];
    if !model.is_empty() {
        require_flag(help, "--model", "Gemini")?;
        args.push("--model".to_string());
        args.push(model.to_string());
    }
    Ok(args)
}

/// `{"response": "…", "stats": …}` on success, `{"error": {...}}` when the
/// CLI itself reports the failure (a non-zero exit with an empty stdout, the
/// far more common case, never reaches this: see `gemini_stderr_sentence`).
fn gemini_envelope(stdout: &str) -> Result<String, String> {
    let value: Value =
        serde_json::from_str(stdout.trim()).map_err(|e| format!("Gemini CLI's output was not JSON: {e}"))?;
    if let Some(error) = value.get("error").filter(|v| !v.is_null()) {
        return Err(process::truncate(&error.to_string(), 300));
    }
    value
        .get("response")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "Gemini CLI's JSON output had no response text.".into())
}

/// The value of the first `"message": "…"` pair in `text`, read as raw
/// characters rather than parsed JSON: Gemini's headless failures put one
/// inside a Node stack trace, not as a document of its own.
fn json_message(text: &str) -> Option<String> {
    const KEY: &str = "\"message\"";
    let after_key = &text[text.find(KEY)? + KEY.len()..];
    let value_text = after_key[after_key.find(':')? + 1..].trim_start();
    let mut message = String::new();
    let mut escaped = false;
    for c in value_text.strip_prefix('"')?.chars() {
        if escaped {
            message.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return Some(message);
        } else {
            message.push(c);
        }
    }
    None
}

/// Gemini's headless failures never reach stdout — only a Node stack trace on
/// stderr says what happened. `UNAUTHENTICATED` is the one case the contract
/// names by its own one-sentence answer; everything else falls back to
/// whatever explanation the trace actually carries.
fn gemini_stderr_sentence(stderr: &str) -> String {
    if stderr.contains("UNAUTHENTICATED") {
        return "Gemini CLI is not signed in. Run `gemini` in a terminal and sign in, then try again.".into();
    }
    let candidate = json_message(stderr)
        .filter(|m| !m.trim().is_empty())
        .or_else(|| stderr.lines().find(|line| line.trim_start().starts_with("Error")).map(str::to_string))
        .unwrap_or_else(|| stderr.to_string());
    let sentence = process::truncate(&candidate, 300);
    if sentence.is_empty() {
        "Gemini CLI exited without an answer.".to_string()
    } else {
        sentence
    }
}

/// `GEMINI_CLI_HOME` first — the installed CLI's own `homedir()` checks it
/// before `os.homedir()` — else `USERPROFILE`. A relative `GEMINI_CLI_HOME`
/// is resolved the same way Gemini resolves it: against its own working
/// folder, `cwd`, not ArchGen's.
fn home_dir(cwd: &Path) -> Option<PathBuf> {
    resolve_home(std::env::var_os("GEMINI_CLI_HOME").as_deref(), std::env::var_os("USERPROFILE").as_deref(), cwd)
}

/// `gemini_cli_home`/`user_profile`, passed in rather than read here, so
/// tests can hand them a constructed value instead of the process's real
/// environment (`locate_claude`'s `path` does the same).
fn resolve_home(gemini_cli_home: Option<&OsStr>, user_profile: Option<&OsStr>, cwd: &Path) -> Option<PathBuf> {
    match gemini_cli_home.filter(|v| !v.is_empty()) {
        Some(value) => {
            let path = PathBuf::from(value);
            Some(if path.is_absolute() { path } else { cwd.join(path) })
        }
        None => user_profile.map(PathBuf::from),
    }
}

/// One stable, always-empty folder: a fresh `TempDir` per request would grow
/// `~/.gemini/projects.json` forever (Gemini registers every cwd it ever runs
/// in there, and never forgets one). A wipe that could not fully empty it
/// (a locked file, another Gemini process using it right now) must fail the
/// request rather than let Gemini run `--skip-trust` (trusted) against
/// whatever is left inside.
fn gemini_cwd() -> Result<PathBuf, String> {
    ensure_empty_dir(&std::env::temp_dir().join("archgen-gemini-cwd"))
}

/// Best-effort wipe, then recreate. A wipe that could not fully empty `path`
/// (a locked file, another Gemini process using it right now) must fail the
/// request rather than let it silently run: `--skip-trust` would then run
/// Gemini as trusted against whatever is left inside.
fn ensure_empty_dir(path: &Path) -> Result<PathBuf, String> {
    let _ = std::fs::remove_dir_all(path);
    std::fs::create_dir_all(path).map_err(|e| format!("creating Gemini CLI's working folder: {e}"))?;
    if std::fs::read_dir(path).map_err(|e| format!("reading Gemini CLI's working folder: {e}"))?.next().is_some() {
        return Err("Gemini CLI's working folder could not be emptied. Close whatever is using it and try again.".into());
    }
    Ok(path.to_path_buf())
}

/// The folder name Gemini filed our prompt under: whatever `projects.json`
/// mapped our lower-cased cwd to, or the cwd's own basename when it never has
/// (a fresh folder, or `projects.json` missing/unreadable).
fn gemini_session_name(home: &Path, cwd: &Path) -> String {
    projects_json_name(home, &cwd.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
}

fn projects_json_name(home: &Path, lowered_cwd: &str) -> Option<String> {
    let text = std::fs::read_to_string(home.join(".gemini").join("projects.json")).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    value.get("projects")?.get(lowered_cwd)?.as_str().map(str::to_string)
}

/// `name` is only ever safe to join onto `.gemini/tmp` or `.gemini/history`
/// when it could be one of Gemini's own slugs: its `slugify()` emits only
/// `[a-z0-9-]`, and ArchGen's own basename fallback (`archgen-gemini-cwd`) is
/// lowercase too, so that alphabet is both necessary and sufficient for a
/// legitimate name. It also rejects everything a syntactic "one normal path
/// component" check alone would miss on Windows: a trailing dot or space
/// (`CreateFile` silently strips those from the final segment, so `"..."` or
/// `"own-ide."` would otherwise resolve to `.gemini/tmp` itself or to another
/// project's folder), an alternate-data-stream colon, and an upper-cased name
/// that NTFS treats as identical to a real project's lower-cased one — on top
/// of every `.`/`..`/separator/absolute path a crafted `projects.json` could
/// name. `projects.json` is a file Gemini writes, but a corrupted or crafted
/// mapping must never turn a best-effort cleanup into deleting an unrelated
/// folder.
fn safe_session_name(name: &str) -> Option<&str> {
    const RESERVED: &[&str] = &[
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1",
        "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    let alphabet = !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    (alphabet && !RESERVED.contains(&name)).then_some(name)
}

/// Best effort and silent: Gemini writes our prompt to `~/.gemini/tmp/<name>`
/// and creates `~/.gemini/history/<name>` no matter what `--approval-mode` or
/// `--skip-trust` say, so this runs after every attempt — success, error or
/// cancel — to remove exactly those two folders for `cwd`, never any other
/// name. A crashed or already-cleaned run simply leaves nothing to remove.
fn cleanup_gemini_session(home: &Path, cwd: &Path) {
    let name = gemini_session_name(home, cwd);
    let Some(name) = safe_session_name(&name) else { return };
    let _ = std::fs::remove_dir_all(home.join(".gemini").join("tmp").join(name));
    let _ = std::fs::remove_dir_all(home.join(".gemini").join("history").join(name));
}

/// Best effort: called at the top of `complete_gemini` (below) so a session
/// orphaned by a *previous* request in this same run gets cleaned before the
/// next one starts, and meant to also run once at start-up next to
/// `process::sweep_stale_temp_dirs` (the integrator's job — see the design
/// doc). ArchGen closing, crashing or being killed mid-request skips the
/// end-of-request clean-up in `complete_gemini`, since nothing waits for that
/// worker thread to finish first, and Gemini has already written the prompt
/// to `~/.gemini/tmp/archgen-gemini-cwd/chats/*.jsonl` (and to
/// `~/.gemini/history`) by the time it is killed — a start-up sweep is the
/// only thing that recovers from that. A run in progress on a second ArchGen
/// instance is unaffected: Gemini recreates both folders on its very next
/// write, and that instance's own clean-up removes them again.
pub fn sweep_stale_gemini_sessions() {
    let cwd = std::env::temp_dir().join("archgen-gemini-cwd");
    if let Some(home) = home_dir(&cwd) {
        cleanup_gemini_session(&home, &cwd);
    }
}

/// Gemini enforces no schema itself, so `schema::by_prompt` puts it in the
/// prompt and gives the model one chance to correct a violation — each
/// attempt is its own `gemini` run, sharing the same argv and cwd.
/// The `Command` each `gemini` attempt runs. `temp` becomes the child's own
/// `TEMP`/`TMP`: on any API failure Gemini CLI writes the whole request —
/// the prompt included — to `<TEMP>/gemini-client-error-*.json`
/// (`reportError`, `os.tmpdir()`), so its temp dir must never be ArchGen's
/// real one. A separate function so tests can inspect the `Command` without
/// spawning anything (`process::run`'s own `scrub_keys` test does the same).
fn gemini_command(path: &Path, args: &[String], cwd: &Path, temp: &Path) -> Command {
    let mut cmd = Command::new(path);
    cmd.args(args).current_dir(cwd).env("TEMP", temp).env("TMP", temp);
    cmd
}

/// Every Gemini request shares one working folder and therefore one session folder under
/// `~/.gemini`; the wipe before and the clean-up after a request must not hit another one mid-run.
static GEMINI_REQUEST: Mutex<()> = Mutex::new(());

fn complete_gemini(route: &Route, request: &Completion, cancel: &Cancel) -> Result<Value, String> {
    let _one_at_a_time = GEMINI_REQUEST.lock().unwrap_or_else(|e| e.into_inner());
    // `Ai::new` sweeps at start-up; this covers a request that died while ArchGen kept running.
    sweep_stale_gemini_sessions();
    let help = cli_help(ProviderKind::GeminiCli)?;
    let cwd = gemini_cwd()?;
    let policy_dir = process::TempDir::new()?;
    let policy_path = policy_dir.write("deny-all.toml", DENY_ALL_POLICY)?;
    let args = gemini_args(&help.text, &route.model, &policy_path)?;
    let limits = process::Limits::new(route.timeout);
    // `policy_dir` is removed on drop, after this closure returns, on
    // success, error and cancel alike.
    let result = schema::by_prompt(request.system, request.user, request.schema, |prompt| {
        let cmd = gemini_command(&help.path, &args, &cwd, policy_dir.path());
        let output = process::run(cmd, prompt, &limits, cancel)?;
        if !output.success {
            return Err(gemini_stderr_sentence(&output.stderr));
        }
        gemini_envelope(&output.stdout)
    });
    if let Some(home) = home_dir(&cwd) {
        cleanup_gemini_session(&home, &cwd);
    }
    result
}

// ── Entry point ──────────────────────────────────────────────────────────────

/// Runs the CLI of `route.kind` and returns the JSON value it answered with.
pub fn complete(route: &Route, request: &Completion, cancel: &Cancel) -> Result<Value, String> {
    match route.kind {
        ProviderKind::ClaudeCli => complete_claude(route, request, cancel),
        ProviderKind::CodexCli => complete_codex(route, request, cancel),
        ProviderKind::GeminiCli => complete_gemini(route, request, cancel),
        _ => Err("not a CLI provider".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real `--help` excerpts captured on this machine, 2026-09-18:
    // Claude Code 2.1.276, Codex CLI 0.155.0, Gemini CLI 0.59.0.
    // The `--restricted` and `--system-prompt-snapshot` descriptions wrap
    // onto lines that themselves start with a dash-led word (`--tools`,
    // `--system-prompt`); they are kept here, exactly as captured, so
    // `documented_flag_does_not_count_a_wrapped_description_line_as_an_option_row`
    // below exercises the real text that once fooled it.
    const CLAUDE_HELP: &str = "Options:
  --json-schema <schema>                JSON Schema for structured output
                                        validation. Example:
                                        {\"type\":\"object\",\"properties\":{\"name\":{\"type\":\"string\"}},\"required\":[\"name\"]}
  --model <model>                       Model for the current session. Provide
                                        an alias for the latest model (e.g.
                                        'fable', 'opus', or 'sonnet') or a
                                        model's full name (e.g.
                                        'claude-fable-5').
  --no-session-persistence              Disable session persistence - sessions
                                        will not be saved to disk and cannot be
                                        resumed (only works with --print)
  --output-format <format>              Output format (only works with --print):
                                        \"text\" (default), \"json\" (single
                                        result), or \"stream-json\" (realtime
                                        streaming) (choices: \"text\", \"json\",
                                        \"stream-json\")
  -p, --print                           Print response and exit (useful for
                                        pipes).
  --restricted                          Restricted mode: removes the built-in
                                        tools that run commands or code (Bash,
                                        PowerShell, REPL and the other
                                        code-running tools) and WebFetch unless
                                        --tools names them, and ignores user,
                                        project and local settings files
                                        (managed settings and --settings still
                                        apply; add --strict-mcp-config to skip
                                        MCP servers too). Also confines the file
                                        tools to the working directories
                                        (--add-dir included), refuses
                                        bypassPermissions, and lets only a
                                        person or the configured permission
                                        handler approve writes to settings, git
                                        and tool-configuration files.
  --strict-mcp-config                   Only use MCP servers from --mcp-config,
                                        ignoring all other MCP configurations
  --system-prompt <prompt>              System prompt to use for the session
  --system-prompt-snapshot <on|off>     Record the system prompt once per
                                        conversation and reuse it verbatim on
                                        every request and resume. on (the
                                        default): the prompt is rendered on the
                                        conversation's first request — a
                                        --system-prompt or
                                        --append-system-prompt included — sent,
                                        and recorded; every later request and
                                        resume sends the record as-is, even when
                                        a later launch passes different text,
                                        until the conversation is compacted.
                                        off: never record; the prompt is
                                        rendered fresh every request (for
                                        iterating on prompt text). No effect
                                        where system-prompt recording is not yet
                                        enabled. (choices: \"on\", \"off\")
  --tools <tools...>                    Specify the list of available tools from
                                        the built-in set. Use \"\" to disable all
                                        tools, \"default\" to use all tools, or
                                        specify tool names (e.g.
                                        \"Bash,Edit,Read\").";

    const CODEX_HELP: &str = "Run Codex non-interactively

Usage: codex exec [OPTIONS] [PROMPT]

Options:
  -c, --config <key=value>
          Override a configuration value that would otherwise be loaded from `~/.codex/config.toml`.

  -m, --model <MODEL>
          Model the agent should use

  -s, --sandbox <SANDBOX_MODE>
          Select the sandbox policy to use when executing model-generated shell commands

          [possible values: read-only, workspace-write, danger-full-access]

  -C, --cd <DIR>
          Tell the agent to use the specified directory as its working root

      --skip-git-repo-check
          Allow running Codex outside a Git repository

      --ephemeral
          Run without persisting session files to disk

      --ignore-user-config
          Do not load `$CODEX_HOME/config.toml`; auth still uses `CODEX_HOME`

      --disable <FEATURE>
          Disable a feature (repeatable). Equivalent to `-c features.<name>=false`

      --output-schema <FILE>
          Path to a JSON Schema file describing the model's final response shape

      --json
          Print events to stdout as JSONL

  -o, --output-last-message <FILE>
          Specifies file where the last message from the agent should be written";

    const GEMINI_HELP: &str = "Usage: gemini [options] [command]

Gemini CLI - Defaults to interactive mode. Use -p/--prompt for non-interactive (headless) mode.

Options:
  -m, --model                     Model  [string]
  -p, --prompt                    Run in non-interactive (headless) mode with the given prompt. Appended to input on stdin (if any).  [string]
      --skip-trust                Trust the current workspace for this session.  [boolean] [default: false]
      --approval-mode             Set the approval mode: default (prompt for approval), auto_edit (auto-approve edit tools), yolo (auto-approve all tools), plan (read-only mode)  [string] [choices: \"default\", \"auto_edit\", \"yolo\", \"plan\"]
      --policy                    Additional policy files or directories to load (comma-separated or multiple --policy)  [array]
      --allowed-mcp-server-names  Allowed MCP server names  [array]
  -o, --output-format             The format of the CLI output.  [string] [choices: \"text\", \"json\", \"stream-json\"]";

    fn without(help: &str, needle: &str) -> String {
        help.lines().filter(|line| !line.contains(needle)).collect::<Vec<_>>().join("\n")
    }

    /// Removes just a substring, keeping the rest of its line (and the flag it
    /// documents) intact — for a value a flag's own line must advertise, such
    /// as Codex's `read-only` sandbox mode or Gemini's `plan` approval mode.
    fn strip(help: &str, needle: &str) -> String {
        help.replace(needle, "")
    }

    // ── documented_flag / require_flag ──────────────────────────────────────

    #[test]
    fn documented_flag_matches_the_flag_itself_not_a_longer_name_that_contains_it() {
        assert!(documented_flag(CLAUDE_HELP, "--tools"));
        assert!(documented_flag(CLAUDE_HELP, "-p"));
        assert!(!documented_flag(CLAUDE_HELP, "--tool"));
        assert!(!documented_flag(CLAUDE_HELP, "--allowed-tools"));
        assert!(!documented_flag(CLAUDE_HELP, "--json-schemas"));
    }

    #[test]
    fn require_flag_fails_closed_with_one_sentence_naming_the_flag() {
        assert_eq!(
            require_flag(&without(CODEX_HELP, "--ignore-user-config"), "--ignore-user-config", "Codex").unwrap_err(),
            "Codex CLI on this computer is too old for ArchGen: it does not support --ignore-user-config."
        );
    }

    #[test]
    fn documented_flag_does_not_count_a_wrapped_description_line_as_an_option_row() {
        // `--restricted`'s description wraps onto a line starting with
        // "--tools ..." and `--system-prompt-snapshot`'s onto one starting
        // with "--system-prompt or"; both must be ignored even though their
        // own option rows are gone.
        let help = without(&without(CLAUDE_HELP, "--tools <tools...>"), "--system-prompt <prompt>");
        assert!(!documented_flag(&help, "--tools"), "a wrapped mention of --tools must not count as documenting it");
        assert!(!documented_flag(&help, "--system-prompt"), "a wrapped mention of --system-prompt must not count as documenting it");
        // The flags that really do stay documented are unaffected.
        assert!(documented_flag(&help, "--restricted"));
        assert!(documented_flag(&help, "--system-prompt-snapshot"));
    }

    #[test]
    fn probe_command_never_runs_in_the_caller_s_own_working_directory() {
        // Before the fix, `probe_help` set no `current_dir` at all, so an npm
        // `.cmd` shim's fallback branch would search ArchGen's own cwd for a
        // bare `node` before PATH — exactly the folder a cloned repository or
        // a "Start in" shortcut could plant one in.
        let dir = process::TempDir::new().unwrap();
        let cmd = probe_command(Path::new("whatever.exe"), &["--help"], dir.path());
        assert_eq!(cmd.get_current_dir(), Some(dir.path()));
    }

    #[test]
    fn probe_command_also_redirects_the_child_s_temp_and_tmp() {
        // Defense in depth: a probe makes no API call today, so it should
        // never hit Gemini's `reportError`, but should a future probe
        // argument reach the network, its child must not be able to write
        // into ArchGen's real %TEMP% either.
        let dir = process::TempDir::new().unwrap();
        let cmd = probe_command(Path::new("gemini.cmd"), &["--help"], dir.path());
        let env: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(name, value)| (name.to_string_lossy().into_owned(), value.map(|v| v.to_string_lossy().into_owned())))
            .collect();
        assert!(env.contains(&("TEMP".to_string(), Some(dir.path().to_string_lossy().into_owned()))), "{env:?}");
        assert!(env.contains(&("TMP".to_string(), Some(dir.path().to_string_lossy().into_owned()))), "{env:?}");
    }

    // ── help probe caching ──────────────────────────────────────────────────

    #[test]
    fn cached_or_probe_never_caches_a_failure_but_caches_the_first_success_for_good() {
        let cache: OnceLock<Result<CliHelp, String>> = OnceLock::new();
        let calls = std::cell::Cell::new(0);

        assert_eq!(
            cached_or_probe(&cache, || {
                calls.set(calls.get() + 1);
                Err("not installed".into())
            })
            .unwrap_err(),
            "not installed"
        );
        assert_eq!(
            cached_or_probe(&cache, || {
                calls.set(calls.get() + 1);
                Err("still not installed".into())
            })
            .unwrap_err(),
            "still not installed",
            "a failed probe must never be cached, so the next request tries again"
        );
        assert_eq!(calls.get(), 2);

        let first = cached_or_probe(&cache, || {
            calls.set(calls.get() + 1);
            Ok(CliHelp { path: PathBuf::from("cli"), text: "help text".into() })
        })
        .unwrap();
        assert_eq!(first.text, "help text");
        assert_eq!(calls.get(), 3);

        // A later call must return the cached value without probing again, even
        // when the closure it is given would answer differently.
        let stale = cached_or_probe(&cache, || {
            calls.set(calls.get() + 1);
            Ok(CliHelp { path: PathBuf::from("other"), text: "different".into() })
        })
        .unwrap();
        assert_eq!(stale.text, "help text", "a cached success must not be re-probed");
        assert_eq!(calls.get(), 3);
    }

    // ── locate ───────────────────────────────────────────────────────────────

    #[test]
    fn locate_claude_exe_prefers_an_exe_anywhere_on_path_over_a_cmd_shim_earlier_on_it() {
        let shim_dir = process::TempDir::new().unwrap();
        std::fs::write(shim_dir.path().join("claude.cmd"), "@echo off").unwrap();
        let exe_dir = process::TempDir::new().unwrap();
        std::fs::write(exe_dir.path().join("claude.exe"), "").unwrap();
        let path = std::env::join_paths([shim_dir.path(), exe_dir.path()]).unwrap();
        assert_eq!(locate_claude_exe(Some(&path)).unwrap(), exe_dir.path().join("claude.exe"));

        let only_shim = std::env::join_paths([shim_dir.path()]).unwrap();
        assert!(locate_claude_exe(Some(&only_shim)).is_none());
    }

    #[test]
    fn locate_claude_falls_back_to_the_profile_local_bin_when_path_has_no_exe() {
        let profile = process::TempDir::new().unwrap();
        std::fs::create_dir_all(profile.path().join(".local").join("bin")).unwrap();
        std::fs::write(profile.path().join(".local").join("bin").join("claude.exe"), "").unwrap();
        let empty = process::TempDir::new().unwrap();
        let path = std::env::join_paths([empty.path()]).unwrap();
        assert_eq!(
            locate_claude(Some(&path), Some(profile.path().as_os_str())).unwrap(),
            profile.path().join(".local").join("bin").join("claude.exe")
        );
    }

    #[test]
    fn is_cmd_shim_looks_only_at_the_extension() {
        assert!(is_cmd_shim(Path::new("codex.cmd")));
        assert!(is_cmd_shim(Path::new("codex.CMD")));
        assert!(is_cmd_shim(Path::new("codex.bat")));
        assert!(!is_cmd_shim(Path::new("claude.exe")));
        assert!(!is_cmd_shim(Path::new("claude")));
    }

    // ── Claude Code ──────────────────────────────────────────────────────────

    #[test]
    fn claude_args_uses_only_documented_flags_in_the_contract_order() {
        let schema = serde_json::json!({"type": "object"});
        let args = claude_args(CLAUDE_HELP, "opus", "be terse", &schema, false).unwrap();
        assert_eq!(
            args,
            [
                "-p",
                "--output-format",
                "json",
                "--json-schema",
                "{\"type\":\"object\"}",
                "--system-prompt",
                "be terse",
                "--tools",
                "",
                "--strict-mcp-config",
                "--no-session-persistence",
                "--model",
                "opus",
            ]
        );
        let no_model = claude_args(CLAUDE_HELP, "", "s", &schema, false).unwrap();
        assert!(!no_model.contains(&"--model".to_string()));
    }

    #[test]
    fn claude_args_folds_crlf_in_the_system_prompt_only_for_a_cmd_shim() {
        let schema = serde_json::json!({});
        let folded = claude_args(CLAUDE_HELP, "", "line one\r\nline two\rline three", &schema, true).unwrap();
        let index = folded.iter().position(|a| a == "--system-prompt").unwrap();
        assert_eq!(folded[index + 1], "line one line two line three");

        let unfolded = claude_args(CLAUDE_HELP, "", "line one\r\nline two", &schema, false).unwrap();
        let index = unfolded.iter().position(|a| a == "--system-prompt").unwrap();
        assert_eq!(unfolded[index + 1], "line one\r\nline two");
    }

    #[test]
    fn claude_args_fails_closed_when_a_required_flag_is_undocumented() {
        for flag in ["-p, --print", "--output-format <format>", "--json-schema <schema>", "--system-prompt <prompt>", "--tools <tools...>", "--strict-mcp-config"] {
            let help = without(CLAUDE_HELP, flag);
            let error = claude_args(&help, "", "s", &serde_json::json!({}), false).unwrap_err();
            assert!(error.contains("too old for ArchGen"), "{flag}: {error}");
        }
    }

    #[test]
    fn claude_args_requires_the_model_flag_only_when_a_model_is_requested() {
        let help = without(CLAUDE_HELP, "--model <model>");
        assert!(claude_args(&help, "", "s", &serde_json::json!({}), false).is_ok());
        assert!(claude_args(&help, "opus", "s", &serde_json::json!({}), false).unwrap_err().contains("--model"));
    }

    #[test]
    fn parse_claude_output_prefers_structured_output_then_falls_back_to_a_json_object_inside_result() {
        let success = r#"{"type":"result","subtype":"success","is_error":false,"result":"{\"ok\":true,\"word\":\"pong\"}","structured_output":{"ok":true,"word":"pong"}}"#;
        assert_eq!(parse_claude_output(success).unwrap(), serde_json::json!({"ok": true, "word": "pong"}));

        let text_only = r#"{"type":"result","is_error":false,"result":"Sure, here you go: {\"ok\":true}"}"#;
        assert_eq!(parse_claude_output(text_only).unwrap(), serde_json::json!({"ok": true}));

        let refused = r#"{"type":"result","is_error":true,"result":"I can't help with that request."}"#;
        assert_eq!(parse_claude_output(refused).unwrap_err(), "I can't help with that request.");

        let nothing = r#"{"type":"result","is_error":false,"result":""}"#;
        assert!(parse_claude_output(nothing).unwrap_err().contains("neither a structured answer nor a result"));

        assert!(parse_claude_output("not json").unwrap_err().contains("was not JSON"));
    }

    #[test]
    fn parse_claude_output_falls_back_to_errors_then_subtype_for_the_error_subtypes_that_carry_no_result() {
        // error_max_structured_output_retries, error_during_execution, error_max_turns and
        // error_max_budget_usd all have no `result` field at all — only `errors`.
        let retries = r#"{"type":"result","subtype":"error_max_structured_output_retries","is_error":true,"errors":["Failed to provide valid structured output after 5 attempts — last StructuredOutput error: missing field `ok`"]}"#;
        assert_eq!(
            parse_claude_output(retries).unwrap_err(),
            "Failed to provide valid structured output after 5 attempts — last StructuredOutput error: missing field `ok`"
        );

        let execution = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["first real reason","stack trace noise"]}"#;
        assert_eq!(parse_claude_output(execution).unwrap_err(), "first real reason");

        // `result` still wins when both are present.
        let both = r#"{"type":"result","subtype":"success","is_error":true,"result":"budget exceeded","errors":["ignored"]}"#;
        assert_eq!(parse_claude_output(both).unwrap_err(), "budget exceeded");

        // Neither `result` nor `errors`: fall back to the subtype, then the generic sentence.
        let subtype_only = r#"{"type":"result","subtype":"error_max_turns","is_error":true}"#;
        assert_eq!(parse_claude_output(subtype_only).unwrap_err(), "Claude Code stopped: error_max_turns");

        let nothing_at_all = r#"{"type":"result","is_error":true}"#;
        assert_eq!(parse_claude_output(nothing_at_all).unwrap_err(), "Claude Code reported an error.");
    }

    #[test]
    fn claude_answer_uses_stderr_only_when_stdout_never_became_the_promised_json() {
        let crashed = process::Output { stdout: "".into(), stderr: "".into(), success: false };
        assert_eq!(claude_answer(&crashed).unwrap_err(), "Claude Code exited without an answer.");

        let old_flag = process::Output { stdout: "garbage, not json".into(), stderr: "error: unknown option '--json-schema'".into(), success: false };
        assert_eq!(claude_answer(&old_flag).unwrap_err(), "error: unknown option '--json-schema'");

        // Non-zero exit but stdout IS the promised JSON: the reply decides, stderr is ignored.
        let is_error = process::Output {
            stdout: r#"{"is_error":true,"result":"budget exceeded"}"#.into(),
            stderr: "irrelevant noise".into(),
            success: false,
        };
        assert_eq!(claude_answer(&is_error).unwrap_err(), "budget exceeded");

        let ok = process::Output { stdout: r#"{"is_error":false,"structured_output":{"ok":true}}"#.into(), stderr: "".into(), success: true };
        assert_eq!(claude_answer(&ok).unwrap(), serde_json::json!({"ok": true}));
    }

    // ── Codex CLI ────────────────────────────────────────────────────────────

    fn known_codex_features() -> Vec<String> {
        TOOL_LIKE_FEATURES.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn codex_args_uses_only_documented_flags_in_the_contract_order() {
        let schema = Path::new("C:\\work\\schema.json");
        let output = Path::new("C:\\work\\output.json");
        let cwd = Path::new("C:\\work");
        let known = known_codex_features();
        let args = codex_args(CODEX_HELP, "gpt", schema, output, cwd, &known).unwrap();
        assert_eq!(&args[..13], [
            "exec",
            "--output-schema", "C:\\work\\schema.json",
            "-o", "C:\\work\\output.json",
            "-s", "read-only",
            "--skip-git-repo-check",
            "-C", "C:\\work",
            "--ignore-user-config",
            "-c", "web_search=disabled",
        ]);
        assert_eq!(args.iter().filter(|a| *a == "--disable").count(), TOOL_LIKE_FEATURES.len());
        for name in ["shell_tool", "unified_exec", "view_image", "apps", "multi_agent", "plugins"] {
            assert!(args.windows(2).any(|w| w[0] == "--disable" && w[1] == name), "missing --disable {name}");
        }
        assert_eq!(args[args.len() - 5..], ["--ephemeral", "--json", "--model", "gpt", "-"]);

        let no_model = codex_args(CODEX_HELP, "", schema, output, cwd, &known).unwrap();
        assert_eq!(no_model.last().unwrap(), "-");
        assert!(!no_model.iter().any(|a| a == "--model"));
    }

    #[test]
    fn codex_args_adds_ephemeral_and_json_only_when_documented() {
        let known = known_codex_features();
        let help = without(&without(CODEX_HELP, "--ephemeral"), "--json");
        let args = codex_args(&help, "", Path::new("s"), Path::new("o"), Path::new("w"), &known).unwrap();
        assert!(!args.iter().any(|a| a == "--ephemeral"));
        assert!(!args.iter().any(|a| a == "--json"));
    }

    #[test]
    fn codex_args_fails_closed_when_a_required_flag_is_undocumented() {
        let known = known_codex_features();
        for flag in [
            "--output-schema <FILE>",
            "-o, --output-last-message <FILE>",
            "-s, --sandbox <SANDBOX_MODE>",
            "--skip-git-repo-check",
            "-C, --cd <DIR>",
            "--ignore-user-config",
            "--disable <FEATURE>",
            "-c, --config <key=value>",
        ] {
            let help = without(CODEX_HELP, flag);
            let error = codex_args(&help, "", Path::new("s"), Path::new("o"), Path::new("w"), &known).unwrap_err();
            assert!(error.contains("too old for ArchGen"), "{flag}: {error}");
        }
    }

    #[test]
    fn codex_args_fails_closed_when_read_only_is_not_an_advertised_sandbox_mode() {
        // `-s, --sandbox` itself stays documented; only the `read-only` value disappears
        // (e.g. a build that renamed or dropped that sandbox policy).
        let help = without(CODEX_HELP, "read-only");
        assert!(documented_flag(&help, "-s"));
        let error = codex_args(&help, "", Path::new("s"), Path::new("o"), Path::new("w"), &known_codex_features()).unwrap_err();
        assert!(error.contains("read-only sandbox"), "{error}");
    }

    #[test]
    fn codex_args_fails_closed_when_shell_tool_or_unified_exec_is_not_a_known_feature() {
        let schema = Path::new("s");
        let output = Path::new("o");
        let cwd = Path::new("w");
        let without_shell: Vec<String> = TOOL_LIKE_FEATURES.iter().filter(|f| **f != "shell_tool").map(|s| s.to_string()).collect();
        assert_eq!(
            codex_args(CODEX_HELP, "", schema, output, cwd, &without_shell).unwrap_err(),
            "Codex CLI on this computer cannot switch its tools off for ArchGen."
        );
        let without_unified: Vec<String> = TOOL_LIKE_FEATURES.iter().filter(|f| **f != "unified_exec").map(|s| s.to_string()).collect();
        assert!(codex_args(CODEX_HELP, "", schema, output, cwd, &without_unified).is_err());
    }

    #[test]
    fn codex_args_only_disables_feature_names_this_install_actually_has() {
        // Shaped like an older Codex (0.150.0): has shell_tool/unified_exec, but
        // predates a couple of the newer names. `--disable` rejects any name it
        // does not recognize, so passing those would fail every request.
        let older: Vec<String> =
            TOOL_LIKE_FEATURES.iter().filter(|f| **f != "unified_exec_tty" && **f != "sleep_tool").map(|s| s.to_string()).collect();
        let args = codex_args(CODEX_HELP, "", Path::new("s"), Path::new("o"), Path::new("w"), &older).unwrap();
        assert_eq!(args.iter().filter(|a| *a == "--disable").count(), TOOL_LIKE_FEATURES.len() - 2);
        for missing in ["unified_exec_tty", "sleep_tool"] {
            assert!(!args.windows(2).any(|w| w[0] == "--disable" && w[1] == missing), "must not --disable an unknown {missing}");
        }
        assert!(args.windows(2).any(|w| w[0] == "--disable" && w[1] == "shell_tool"));
    }

    #[test]
    fn parse_feature_names_takes_the_first_token_of_each_line() {
        let stdout = "shell_tool                               stable             true\nweb_search_cached                        deprecated         false\nstandalone_web_search                    under development  false\n";
        assert_eq!(parse_feature_names(stdout), vec!["shell_tool", "web_search_cached", "standalone_web_search"]);
    }

    #[test]
    fn codex_answer_parses_the_bare_json_object_and_rejects_garbage_or_a_truncated_file() {
        assert_eq!(codex_answer(br#"{"ok":true}"#).unwrap(), serde_json::json!({"ok": true}));
        assert_eq!(codex_answer(b"  {\"ok\":true}\n").unwrap(), serde_json::json!({"ok": true}));
        assert!(codex_answer(b"Sorry, I can't do that.").unwrap_err().contains("was not JSON"));
        assert!(codex_answer(br#"{"ok":tr"#).unwrap_err().contains("was not JSON"));
        assert!(codex_answer(b"").unwrap_err().contains("was not JSON"));
    }

    #[test]
    fn codex_json_failure_prefers_the_last_turn_failed_over_a_transient_error_event() {
        let retry_then_fail = "{\"type\":\"thread.started\"}\n\
             {\"type\":\"error\",\"message\":\"Reconnecting... waiting for network\"}\n\
             {\"type\":\"turn.failed\",\"error\":{\"message\":\"unexpected status 401 Unauthorized\"}}\n";
        assert_eq!(codex_json_failure(retry_then_fail).unwrap(), "unexpected status 401 Unauthorized");

        let error_only = "{\"type\":\"thread.started\"}\n{\"type\":\"error\",\"message\":\"unexpected status 401 Unauthorized\"}\n";
        assert_eq!(codex_json_failure(error_only).unwrap(), "unexpected status 401 Unauthorized");

        assert!(codex_json_failure("not json at all").is_none());
        assert!(codex_json_failure("{\"type\":\"turn.completed\"}").is_none());
    }

    #[test]
    fn codex_failure_never_quotes_the_startup_banner_or_the_echoed_prompt() {
        assert_eq!(codex_failure("", ""), "Codex CLI exited without an answer.");
        assert_eq!(codex_failure("", "   \n  "), "Codex CLI exited without an answer.");
        assert_eq!(codex_failure("", "error: unrecognized argument"), "error: unrecognized argument");

        // The real shape of stderr on a non-zero exit: a startup banner, then
        // the whole echoed stdin prompt, then the actual reason. The banner
        // and the document text it carries must never be quoted.
        let banner_and_prompt = "OpenAI Codex v0.155.0\n--------\nworkdir: C:\\Users\\ondre\\AppData\\Local\\Temp\\archgen-ai-12345-0-abcd\n\
             model: gpt-5\nprovider: openai\n--------\nuser\nYou are an editor of software-architecture documentation. SYSTEM-PROMPT-MARKER\n\n\
             Rewrite this. USER-PROMPT-MARKER {\"document\":{}}\nERROR: {\"error\":{\"message\":\"The model `nope-9` does not exist\"}}";
        let reason = codex_failure("", banner_and_prompt);
        assert_eq!(reason, "ERROR: {\"error\":{\"message\":\"The model `nope-9` does not exist\"}}");
        assert!(!reason.contains("workdir:") && !reason.contains("SYSTEM-PROMPT-MARKER"));

        // No ERROR:/Error:/error: line anywhere: never fall back to the banner either.
        assert_eq!(codex_failure("", "just noise\nmore noise"), "Codex CLI exited without an answer.");

        // `--json`'s event stream, when present, wins over stderr entirely.
        let json_stdout = "{\"type\":\"turn.failed\",\"error\":{\"message\":\"unexpected status 401 Unauthorized\"}}\n";
        assert_eq!(codex_failure(json_stdout, banner_and_prompt), "unexpected status 401 Unauthorized");
    }

    #[test]
    fn read_bounded_refuses_a_file_larger_than_the_limit() {
        let dir = process::TempDir::new().unwrap();
        let path = dir.write("big.json", &"x".repeat(20)).unwrap();
        assert_eq!(read_bounded(&path, 20).unwrap().len(), 20);
        assert!(read_bounded(&path, 19).unwrap_err().contains("larger than ArchGen accepts"));
    }

    // ── Gemini CLI ───────────────────────────────────────────────────────────

    #[test]
    fn gemini_command_points_the_child_s_temp_and_tmp_at_the_given_folder() {
        // Gemini CLI writes the full prompt to `<TEMP>/gemini-client-error-*.json`
        // on every API failure; before this, the child inherited ArchGen's real
        // `%TEMP%` and those reports leaked the selected document content there.
        let temp = Path::new(r"C:\work\archgen-ai-policy-dir");
        let cmd = gemini_command(Path::new("gemini.cmd"), &["--prompt".to_string()], Path::new(r"C:\work\cwd"), temp);
        let env: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(name, value)| (name.to_string_lossy().into_owned(), value.map(|v| v.to_string_lossy().into_owned())))
            .collect();
        assert!(env.contains(&("TEMP".to_string(), Some(temp.to_string_lossy().into_owned()))), "{env:?}");
        assert!(env.contains(&("TMP".to_string(), Some(temp.to_string_lossy().into_owned()))), "{env:?}");
        assert_eq!(cmd.get_current_dir(), Some(Path::new(r"C:\work\cwd")));
    }

    #[test]
    fn gemini_args_uses_only_documented_flags_in_the_contract_order() {
        let policy = Path::new("C:\\work\\deny-all.toml");
        let args = gemini_args(GEMINI_HELP, "flash", policy).unwrap();
        assert_eq!(
            args,
            [
                "--prompt", "Follow the task supplied on stdin. Do not use tools.",
                "--approval-mode", "plan",
                "--skip-trust",
                "--policy", "C:\\work\\deny-all.toml",
                "--allowed-mcp-server-names", "archgen-none",
                "-o", "json",
                "--model", "flash",
            ]
        );
        let no_model = gemini_args(GEMINI_HELP, "", policy).unwrap();
        assert!(!no_model.iter().any(|a| a == "--model"));
    }

    #[test]
    fn gemini_args_fails_closed_when_a_required_flag_is_undocumented() {
        for flag in ["-p, --prompt", "--approval-mode", "--skip-trust", "--policy", "--allowed-mcp-server-names", "-o, --output-format"] {
            let help = without(GEMINI_HELP, flag);
            let error = gemini_args(&help, "", Path::new("p")).unwrap_err();
            assert!(error.contains("too old for ArchGen"), "{flag}: {error}");
        }
    }

    #[test]
    fn gemini_args_fails_closed_when_prompt_is_not_documented_as_appended_to_stdin() {
        let help = strip(GEMINI_HELP, "stdin");
        assert!(documented_flag(&help, "--prompt")); // the flag itself stays documented
        let error = gemini_args(&help, "", Path::new("p")).unwrap_err();
        assert!(error.contains("appending the prompt to stdin"), "{error}");
    }

    #[test]
    fn gemini_args_fails_closed_when_plan_mode_is_not_documented() {
        let help = strip(GEMINI_HELP, "plan");
        assert!(documented_flag(&help, "--approval-mode"));
        let error = gemini_args(&help, "", Path::new("p")).unwrap_err();
        assert!(error.contains("--approval-mode plan"), "{error}");
    }

    #[test]
    fn gemini_args_fails_closed_when_json_output_is_not_documented() {
        let help = strip(GEMINI_HELP, "\"json\"");
        assert!(documented_flag(&help, "-o"));
        let error = gemini_args(&help, "", Path::new("p")).unwrap_err();
        assert!(error.contains("--output-format json"), "{error}");
    }

    #[test]
    fn gemini_envelope_extracts_response_and_surfaces_error_or_garbage() {
        assert_eq!(gemini_envelope(r#"{"response":"{\"ok\":true}","stats":{}}"#).unwrap(), r#"{"ok":true}"#);
        assert_eq!(gemini_envelope(r#"{"response":"I can't help with that."}"#).unwrap(), "I can't help with that.");
        assert!(gemini_envelope(r#"{"error":{"code":429,"message":"quota exceeded"}}"#).unwrap_err().contains("quota exceeded"));
        assert!(gemini_envelope(r#"{"response": "trunc"#).unwrap_err().contains("was not JSON"));
        assert!(gemini_envelope("not json").unwrap_err().contains("was not JSON"));
        assert!(gemini_envelope(r#"{"stats":{}}"#).unwrap_err().contains("no response text"));
    }

    #[test]
    fn json_message_reads_the_first_message_value_through_escapes() {
        assert_eq!(json_message(r#"blah {"message":"quota \"exceeded\" today"} blah"#).unwrap(), "quota \"exceeded\" today");
        assert_eq!(json_message(r#"{"a":{"message": "spaced"}}"#).unwrap(), "spaced");
        assert!(json_message("no message field here").is_none());
        assert!(json_message(r#"{"message": not_a_string}"#).is_none());
    }

    #[test]
    fn gemini_stderr_sentence_maps_the_real_401_captured_on_this_machine() {
        let real = r#"Error generating content via API. Full report available at: C:\Users\ondre\AppData\Local\Temp\gemini-client-error-generateJson-api-2026-09-18T13-28-41-157Z.json _ApiError: {"error":{"code":401,"message":"Request had invalid authentication credentials. Expected OAuth 2 access token, login cookie or other valid authentication credential.","status":"UNAUTHENTICATED","details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"ACCESS_TOKEN_TYPE_UNSUPPORTED"}]}}
    at throwErrorIfNotOK (file:///C:/Users/ondre/AppData/Roaming/npm/node_modules/@google/gemini-cli/bundle/chunk-YSBB75DZ.js:267315:24)"#;
        assert_eq!(
            gemini_stderr_sentence(real),
            "Gemini CLI is not signed in. Run `gemini` in a terminal and sign in, then try again."
        );
    }

    #[test]
    fn gemini_stderr_sentence_falls_back_to_a_message_field_then_an_error_line_then_raw_text() {
        assert_eq!(gemini_stderr_sentence(r#"Node stack noise {"message":"model overloaded, try again"} more noise"#), "model overloaded, try again");
        assert_eq!(gemini_stderr_sentence("Warning: something\nError: the model is unavailable\n    at foo.js:1:1"), "Error: the model is unavailable");
        assert_eq!(gemini_stderr_sentence("totally unstructured failure text"), "totally unstructured failure text");
    }

    #[test]
    fn gemini_stderr_sentence_never_returns_a_blank_error() {
        assert_eq!(gemini_stderr_sentence(""), "Gemini CLI exited without an answer.");
        assert_eq!(gemini_stderr_sentence("   \r\n  "), "Gemini CLI exited without an answer.");
        // A blank `"message"` value must not win and go straight through as an
        // empty error: it falls through to the next candidate (here, the raw
        // stderr) instead of being returned as-is.
        assert_eq!(gemini_stderr_sentence(r#"{"error":{"message":""}}"#), r#"{"error":{"message":""}}"#);
    }

    #[test]
    fn a_huge_stack_trace_is_reduced_to_one_sentence_of_at_most_300_characters() {
        // No "message" field and no line starting with "Error": every fallback
        // in turn bottoms out at the raw-text branch, which still must be capped.
        let huge = "    at frame (native)\n".repeat(2000);
        assert!(huge.chars().count() > 300);
        assert!(gemini_stderr_sentence(&huge).chars().count() <= 301);

        let crashed = process::Output { stdout: "garbage, not json".into(), stderr: huge.clone(), success: false };
        assert!(claude_answer(&crashed).unwrap_err().chars().count() <= 301);

        assert!(codex_failure("", &huge).chars().count() <= 301);
        // A huge stderr that does end in a long ERROR: line must still be capped.
        let huge_error = format!("{huge}ERROR: {}", "x".repeat(500));
        assert!(codex_failure("", &huge_error).chars().count() <= 301);
    }

    // ── Gemini session cleanup ───────────────────────────────────────────────

    fn write_session(home: &Path, name: &str) {
        std::fs::create_dir_all(home.join(".gemini").join("tmp").join(name).join("chats")).unwrap();
        std::fs::write(home.join(".gemini").join("tmp").join(name).join("chats").join("s.jsonl"), "{}").unwrap();
        std::fs::create_dir_all(home.join(".gemini").join("history").join(name)).unwrap();
    }

    #[test]
    fn gemini_session_name_comes_from_projects_json_or_falls_back_to_the_cwd_basename() {
        let home = process::TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join(".gemini")).unwrap();
        std::fs::write(
            home.path().join(".gemini").join("projects.json"),
            r#"{"projects":{"c:\\fake\\cwd":"mapped-name"}}"#,
        )
        .unwrap();
        assert_eq!(gemini_session_name(home.path(), Path::new(r"C:\Fake\Cwd")), "mapped-name");
        assert_eq!(gemini_session_name(home.path(), Path::new(r"C:\Other\unmapped-cwd")), "unmapped-cwd");
    }

    #[test]
    fn safe_session_name_accepts_only_gemini_s_own_slug_alphabet() {
        assert_eq!(safe_session_name("archgen-gemini-cwd"), Some("archgen-gemini-cwd"));
        assert_eq!(safe_session_name("own-ide"), Some("own-ide"));
        assert_eq!(safe_session_name("project-42"), Some("project-42"));
        for bad in [
            "",
            ".",
            "..",
            "../evil",
            "..\\evil",
            "a/b",
            "a\\b",
            "/etc",
            "\\Windows",
            "C:\\Windows\\System32",
            "C:evil",
            // Windows normalises a trailing dot or space off the final path
            // segment, so each of these would otherwise resolve to
            // `.gemini/tmp`/`.gemini/history` itself.
            " ",
            "...",
            ". ",
            ".. ",
            " .",
            "....",
            // Same normalisation, but landing on another project's real folder.
            "own-ide.",
            "own-ide ",
            // An alternate-data-stream suffix still names the plain folder before it.
            "own-ide::$INDEX_ALLOCATION",
            // NTFS is case-insensitive: this is the very folder "own-ide" names.
            "OWN-IDE",
            // Windows reserved device names.
            "con",
            "CON",
            "nul",
            "com1",
            "lpt1",
        ] {
            assert_eq!(safe_session_name(bad), None, "{bad:?} must be refused");
        }
    }

    #[test]
    fn cleanup_never_escapes_gemini_tmp_or_history_when_projects_json_maps_our_cwd_to_a_traversal() {
        let root = process::TempDir::new().unwrap();
        let home = root.path().join("home");
        std::fs::create_dir_all(home.join(".gemini")).unwrap();
        std::fs::write(
            home.join(".gemini").join("projects.json"),
            serde_json::json!({"projects": {"c:\\fake\\cwd": "../../../escaped"}}).to_string(),
        )
        .unwrap();
        // What an unguarded `.join(name)` would have reached from `home/.gemini/tmp`:
        // three levels up is `root`, so this is a sibling of `home` itself.
        let escaped = root.path().join("escaped");
        std::fs::create_dir_all(&escaped).unwrap();
        write_session(&home, "someone-elses-project");

        cleanup_gemini_session(&home, Path::new(r"C:\Fake\Cwd"));

        assert!(escaped.exists(), "a `..`-laden mapped name must never escape .gemini/tmp or .gemini/history");
        assert!(home.join(".gemini").join("tmp").join("someone-elses-project").exists());
        assert!(home.join(".gemini").join("history").join("someone-elses-project").exists());
    }

    #[test]
    fn cleanup_never_deletes_a_directory_named_by_an_absolute_mapped_name() {
        let root = process::TempDir::new().unwrap();
        let home = root.path().join("home");
        std::fs::create_dir_all(home.join(".gemini")).unwrap();
        let trap = root.path().join("trap");
        std::fs::create_dir_all(&trap).unwrap();
        std::fs::write(
            home.join(".gemini").join("projects.json"),
            serde_json::json!({"projects": {"c:\\fake\\cwd": trap.to_string_lossy()}}).to_string(),
        )
        .unwrap();

        cleanup_gemini_session(&home, Path::new(r"C:\Fake\Cwd"));

        assert!(trap.exists(), "an absolute mapped name must never be joined into a real path and deleted");
    }

    #[test]
    fn cleanup_removes_only_our_cwds_two_folders_and_never_panics_on_a_missing_home() {
        let home = process::TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join(".gemini")).unwrap();
        std::fs::write(
            home.path().join(".gemini").join("projects.json"),
            r#"{"projects":{"c:\\fake\\archgen-gemini-cwd":"archgen-gemini-cwd"}}"#,
        )
        .unwrap();
        write_session(home.path(), "archgen-gemini-cwd");
        write_session(home.path(), "someone-elses-project");

        cleanup_gemini_session(home.path(), Path::new(r"C:\Fake\archgen-gemini-cwd"));

        assert!(!home.path().join(".gemini").join("tmp").join("archgen-gemini-cwd").exists());
        assert!(!home.path().join(".gemini").join("history").join("archgen-gemini-cwd").exists());
        assert!(home.path().join(".gemini").join("tmp").join("someone-elses-project").exists());
        assert!(home.path().join(".gemini").join("history").join("someone-elses-project").exists());

        // Nothing left for this cwd: a second call is a silent no-op, and a
        // home directory that does not exist at all must not panic either.
        cleanup_gemini_session(home.path(), Path::new(r"C:\Fake\archgen-gemini-cwd"));
        cleanup_gemini_session(Path::new(r"Z:\does-not-exist"), Path::new(r"C:\Fake\archgen-gemini-cwd"));
    }

    #[test]
    fn resolve_home_prefers_a_non_empty_gemini_cli_home_and_resolves_a_relative_value_against_cwd() {
        let cwd = Path::new(r"C:\Temp\archgen-gemini-cwd");
        assert_eq!(
            resolve_home(Some(OsStr::new(r"D:\custom\gemini-home")), Some(OsStr::new(r"C:\Users\ondre")), cwd),
            Some(PathBuf::from(r"D:\custom\gemini-home"))
        );
        // A relative GEMINI_CLI_HOME resolves against Gemini's own working
        // folder, the way the installed CLI resolves it — not ArchGen's.
        assert_eq!(resolve_home(Some(OsStr::new("relative-home")), None, cwd), Some(cwd.join("relative-home")));
        // An empty value falls back to USERPROFILE, same as unset.
        assert_eq!(
            resolve_home(Some(OsStr::new("")), Some(OsStr::new(r"C:\Users\ondre")), cwd),
            Some(PathBuf::from(r"C:\Users\ondre"))
        );
        assert_eq!(resolve_home(None, Some(OsStr::new(r"C:\Users\ondre")), cwd), Some(PathBuf::from(r"C:\Users\ondre")));
        assert_eq!(resolve_home(None, None, cwd), None);
    }

    #[test]
    fn ensure_empty_dir_recreates_an_already_empty_or_missing_folder() {
        let root = process::TempDir::new().unwrap();
        let target = root.path().join("cwd");
        assert_eq!(ensure_empty_dir(&target).unwrap(), target);
        assert!(target.is_dir());
        // Already empty on a second call.
        assert_eq!(ensure_empty_dir(&target).unwrap(), target);
    }

    #[test]
    #[cfg(windows)]
    fn ensure_empty_dir_fails_closed_when_a_locked_file_survives_the_wipe() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = process::TempDir::new().unwrap();
        let target = root.path().join("cwd");
        std::fs::create_dir_all(&target).unwrap();
        // Opened with share_mode(0) and kept open for the whole test: no
        // other handle, deletion included, may touch this file while it
        // stays open, exactly as a running Gemini process using this folder
        // as its cwd would hold its own files.
        let _locked =
            std::fs::OpenOptions::new().write(true).create(true).share_mode(0).open(target.join("locked.txt")).unwrap();

        let error = ensure_empty_dir(&target).unwrap_err();

        assert!(error.contains("could not be emptied"), "{error}");
    }

    // ── Live checks: real installed CLIs, no mocking. `#[ignore]`d — run with
    // `cargo test --offline live_<provider> -- --ignored --nocapture`. Each
    // passes with a printed message when the CLI is missing or not signed in,
    // and panics on anything else: an argument ArchGen itself got wrong, a
    // broken parser, a timeout — none of those are "unavailable".

    fn tiny_schema() -> Value {
        serde_json::json!({"type":"object","additionalProperties":false,"required":["ok"],"properties":{"ok":{"type":"boolean"}}})
    }

    /// The only errors a live check may treat as "not installed or not signed
    /// in" rather than a regression: each CLI's own not-authenticated wording.
    fn is_unavailable(error: &str) -> bool {
        let lower = error.to_lowercase();
        ["not signed in", "sign in", "log in", "login", "401", "unauthorized", "api key"]
            .iter()
            .any(|needle| lower.contains(needle))
    }

    fn list_names(dir: &Path) -> std::collections::BTreeSet<String> {
        std::fs::read_dir(dir)
            .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default()
    }

    fn gemini_client_error_files() -> std::collections::BTreeSet<String> {
        list_names(&std::env::temp_dir()).into_iter().filter(|name| name.starts_with("gemini-client-error-")).collect()
    }

    #[test]
    #[ignore]
    fn live_claude_cli() {
        if locate(ProviderKind::ClaudeCli).is_none() {
            println!("Claude CLI is not installed; skipping live_claude_cli.");
            return;
        }
        let schema = tiny_schema();
        let route = Route { kind: ProviderKind::ClaudeCli, model: String::new(), api_key: None, ollama_url: String::new(), timeout: Duration::from_secs(60) };
        let request = Completion { system: "You are a test assistant. Do not use tools.", user: "Set ok to true.", schema: &schema, max_output_tokens: super::super::TEST_MAX_OUTPUT_TOKENS, probe: true };
        match super::super::complete(&route, &request, &Cancel::new()) {
            Ok(value) => assert_eq!(value["ok"], serde_json::json!(true)),
            Err(e) if is_unavailable(&e) => println!("Claude CLI is not signed in; skipping live_claude_cli: {e}"),
            Err(e) => panic!("Claude CLI's request failed (not a not-signed-in error, so this is a real regression): {e}"),
        }
    }

    #[test]
    #[ignore]
    fn live_codex_cli() {
        if locate(ProviderKind::CodexCli).is_none() {
            println!("Codex CLI is not installed; skipping live_codex_cli.");
            return;
        }
        // An unguessable canary, planted outside Codex's own empty cwd: only a
        // working file-reading tool can make it come back in the answer, so
        // its presence proves a tool ran despite every tool-like feature being
        // disabled — unlike the machine's own username, this cannot be a
        // lucky guess, and it does not depend on that username's casing.
        let canary_dir = process::TempDir::new().unwrap();
        let canary = uuid::Uuid::new_v4().simple().to_string();
        let canary_path = canary_dir.write("canary.txt", &canary).unwrap();

        let schema = serde_json::json!({"type":"object","additionalProperties":false,"required":["result"],"properties":{"result":{"type":"string"}}});
        let route = Route { kind: ProviderKind::CodexCli, model: String::new(), api_key: None, ollama_url: String::new(), timeout: Duration::from_secs(90) };
        let user = format!("Print the exact contents of the file {} into the result field.", canary_path.display());
        let request = Completion {
            system: "You are a test assistant. Answer only the supplied request; do not use tools.",
            user: &user,
            schema: &schema,
            max_output_tokens: super::super::TEST_MAX_OUTPUT_TOKENS,
            probe: true,
        };
        match super::super::complete(&route, &request, &Cancel::new()) {
            Ok(value) => {
                let result = value["result"].as_str().unwrap_or_default();
                assert!(
                    !result.contains(&canary),
                    "Codex answered with the canary file's content, so a tool ran despite every tool-like feature being disabled: {result}"
                );
                println!("Codex answered without a tool call: {result}");
            }
            Err(e) if is_unavailable(&e) => println!("Codex CLI is not signed in; skipping live_codex_cli: {e}"),
            Err(e) => panic!("Codex CLI's request failed (not a not-signed-in error, so this is a real regression): {e}"),
        }
    }

    #[test]
    #[ignore]
    fn live_gemini_cli() {
        if locate(ProviderKind::GeminiCli).is_none() {
            println!("Gemini CLI is not installed; skipping live_gemini_cli.");
            return;
        }
        // On any API failure Gemini CLI writes the whole request — this
        // prompt included — to `<TEMP>/gemini-client-error-*.json`
        // (`reportError`, `os.tmpdir()`); `complete_gemini` must redirect the
        // child's TEMP/TMP so none of those land in the real one. This
        // machine's Gemini CLI is signed out, so the 401 failure path that
        // triggers this is the free, common case.
        let error_reports_before = gemini_client_error_files();
        let cwd = std::env::temp_dir().join("archgen-gemini-cwd");
        let home = home_dir(&cwd);
        let tmp_before = home.as_deref().map(|h| list_names(&h.join(".gemini").join("tmp"))).unwrap_or_default();
        let history_before = home.as_deref().map(|h| list_names(&h.join(".gemini").join("history"))).unwrap_or_default();

        let schema = tiny_schema();
        let route = Route { kind: ProviderKind::GeminiCli, model: String::new(), api_key: None, ollama_url: String::new(), timeout: Duration::from_secs(60) };
        let request = Completion { system: "Answer only the supplied request.", user: "Set ok to true.", schema: &schema, max_output_tokens: super::super::TEST_MAX_OUTPUT_TOKENS, probe: true };
        let outcome = super::super::complete(&route, &request, &Cancel::new());

        let new_reports: Vec<_> = gemini_client_error_files().difference(&error_reports_before).cloned().collect();
        assert!(new_reports.is_empty(), "Gemini CLI wrote a client-error report carrying our prompt into the real %TEMP%: {new_reports:?}");

        // Independent of `gemini_session_name`: whatever name Gemini actually
        // filed our prompt under, neither folder may have grown net of the run.
        if let Some(home) = &home {
            let tmp_after = list_names(&home.join(".gemini").join("tmp"));
            let history_after = list_names(&home.join(".gemini").join("history"));
            assert!(tmp_after.is_subset(&tmp_before), "a new folder appeared under ~/.gemini/tmp and was not cleaned up: {tmp_after:?}");
            assert!(
                history_after.is_subset(&history_before),
                "a new folder appeared under ~/.gemini/history and was not cleaned up: {history_after:?}"
            );
            // Secondary, specific check: the name `cleanup_gemini_session` itself derives is gone too.
            let name = gemini_session_name(home, &cwd);
            assert!(!home.join(".gemini").join("tmp").join(&name).exists());
            assert!(!home.join(".gemini").join("history").join(&name).exists());
        }

        match outcome {
            Ok(value) => assert_eq!(value["ok"], serde_json::json!(true)),
            Err(e) if is_unavailable(&e) => println!("Gemini CLI is not signed in; skipping the answer check: {e}"),
            Err(e) => panic!("Gemini CLI's request failed (not a not-signed-in error, so this is a real regression): {e}"),
        }
    }
}
