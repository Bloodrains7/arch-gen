//! Running a provider process: bounded output, a deadline, and cancellation that
//! takes the whole process tree (an npm `.cmd` shim's real work is a grandchild).
//!
//! On Windows every child lives in a Job Object with KILL_ON_JOB_CLOSE: ending the
//! request, or ArchGen itself, ends every descendant — including ones whose parent
//! already exited, which `taskkill /T` cannot find.
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime},
};

/// The exact error of a cancelled request; callers compare against it.
pub const CANCELLED: &str = "cancelled";

const TEMP_PREFIX: &str = "archgen-ai-";
/// After the process exits, how long to wait for output still held open by a descendant.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// Shared between the job registry and the thread that runs the provider.
/// Cancelling only raises a flag (safe on the UI thread); `run` does the killing.
#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Idempotent. A running process is stopped within a few tens of milliseconds.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

pub struct Limits {
    pub timeout: Duration,
    /// More than this on stdout fails the request.
    pub max_stdout: usize,
    /// Only the first bytes of stderr are kept; the rest is read and dropped.
    pub max_stderr: usize,
}

impl Limits {
    pub fn new(timeout: Duration) -> Self {
        Self { timeout, max_stdout: 8 * 1024 * 1024, max_stderr: 64 * 1024 }
    }
}

#[derive(Debug)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

#[cfg(windows)]
pub fn hide_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

#[cfg(not(windows))]
pub fn hide_console(_cmd: &mut Command) {}

#[cfg(windows)]
mod tree {
    use std::{os::windows::io::AsRawHandle, process::Child};
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{CloseHandle, HANDLE},
            System::JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
        },
    };

    pub struct Job(HANDLE);
    // A job handle is a plain kernel handle; it is only used from the thread that runs the child.
    unsafe impl Send for Job {}

    impl Job {
        /// `None` when the child could not be put into a job; the caller falls back to taskkill.
        pub fn adopt(child: &Child) -> Option<Self> {
            unsafe {
                let job = Job(CreateJobObjectW(None, PCWSTR::null()).ok()?);
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(info).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
                .ok()?;
                AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle() as isize)).ok()?;
                Some(job)
            }
        }

        pub fn terminate(&self) {
            unsafe {
                let _ = TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

#[cfg(not(windows))]
mod tree {
    pub struct Job;
    impl Job {
        pub fn adopt(_child: &std::process::Child) -> Option<Self> {
            None
        }
        pub fn terminate(&self) {}
    }
}

/// Best effort: a process that already exited is the outcome that was asked for.
pub fn kill_tree(pid: u32) {
    #[cfg(windows)]
    let mut cmd = {
        let mut cmd = Command::new("taskkill");
        cmd.args(["/PID", &pid.to_string(), "/T", "/F"]);
        cmd
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut cmd = Command::new("kill");
        cmd.args(["-9", &pid.to_string()]);
        cmd
    };
    hide_console(&mut cmd);
    let _ = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

fn stop(child: &mut Child, job: Option<&tree::Job>) {
    match job {
        Some(job) => job.terminate(),
        None => kill_tree(child.id()),
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// `program` on PATH, tried the way a shell would: `.exe`, `.cmd`, `.bat`, then bare.
pub fn locate(program: &str) -> Option<PathBuf> {
    locate_in(program, &std::env::var_os("PATH")?)
}

fn locate_in(program: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    let extensions: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
    std::env::split_paths(path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| extensions.iter().map(move |ext| dir.join(format!("{program}{ext}"))))
        .find(|candidate| candidate.is_file())
}

/// No provider process inherits an API key from ArchGen's environment: a key reaches
/// curl only through its stdin config, and an agent CLI uses its own login. A caller
/// that really wants a child to see one sets it on the `Command` explicitly.
fn scrub_keys(cmd: &mut Command) {
    let explicit: Vec<std::ffi::OsString> = cmd.get_envs().map(|(name, _)| name.to_owned()).collect();
    for name in super::ProviderKind::ALL.iter().flat_map(|kind| kind.key_variables()) {
        if !explicit.iter().any(|set| set.eq_ignore_ascii_case(name)) {
            cmd.env_remove(name);
        }
    }
}

struct Pipe {
    bytes: Arc<Mutex<Vec<u8>>>,
    done: mpsc::Receiver<()>,
}

/// Reads on its own thread so a full pipe never blocks the child. Keeps at most `limit`
/// bytes; beyond that either raises `overflow` and stops (stdout) or keeps draining (stderr).
fn read_pipe(mut source: impl Read + Send + 'static, limit: usize, overflow: Option<Arc<AtomicBool>>) -> Pipe {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (finished, done) = mpsc::channel();
    let shared = bytes.clone();
    std::thread::spawn(move || {
        let mut chunk = [0u8; 16 * 1024];
        loop {
            match source.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    let mut kept = shared.lock().unwrap_or_else(|e| e.into_inner());
                    let room = limit.saturating_sub(kept.len());
                    kept.extend_from_slice(&chunk[..count.min(room)]);
                    if count > room {
                        if let Some(overflow) = &overflow {
                            overflow.store(true, Ordering::SeqCst);
                            break;
                        }
                    }
                }
            }
        }
        let _ = finished.send(());
    });
    Pipe { bytes, done }
}

impl Pipe {
    fn take(&self, wait: Duration) -> Vec<u8> {
        let _ = self.done.recv_timeout(wait);
        std::mem::take(&mut *self.bytes.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// Runs `cmd` with `stdin` as its whole input and waits for it to finish.
/// `Ok` also for a non-zero exit: the caller knows how its provider reports errors.
/// `Err(CANCELLED)` when cancelled. Cancel, the deadline and oversized stdout stop the
/// whole process tree and return at once, whatever still holds the pipes.
pub fn run(mut cmd: Command, stdin: &str, limits: &Limits, cancel: &Cancel) -> Result<Output, String> {
    if cancel.is_cancelled() {
        return Err(CANCELLED.into());
    }
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    hide_console(&mut cmd);
    scrub_keys(&mut cmd);
    // An npm `.cmd` shim starts `node` through cmd.exe, which looks in the working folder first.
    cmd.env("NoDefaultCurrentDirectoryInExePath", "1");
    let mut child = cmd.spawn().map_err(|e| format!("The provider could not be started: {e}"))?;
    let job = tree::Job::adopt(&child);
    let mut input = child.stdin.take().expect("piped stdin");
    let text = stdin.to_owned();
    // Never joined: it ends when the input is written or the pipe breaks.
    std::thread::spawn(move || {
        let _ = input.write_all(text.as_bytes());
    });
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout = read_pipe(child.stdout.take().expect("piped stdout"), limits.max_stdout, Some(overflow.clone()));
    let stderr = read_pipe(child.stderr.take().expect("piped stderr"), limits.max_stderr, None);

    let started = Instant::now();
    let status = loop {
        let failure = if cancel.is_cancelled() {
            Some(CANCELLED.to_string())
        } else if overflow.load(Ordering::SeqCst) {
            Some("The provider printed more output than ArchGen accepts.".into())
        } else if started.elapsed() > limits.timeout {
            Some(format!("The provider did not answer within {} s.", limits.timeout.as_secs()))
        } else {
            None
        };
        if let Some(failure) = failure {
            stop(&mut child, job.as_ref());
            return Err(failure);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => {
                stop(&mut child, job.as_ref());
                return Err(format!("waiting for the provider: {e}"));
            }
        }
    };
    let out = stdout.take(DRAIN_GRACE);
    let err = stderr.take(Duration::from_millis(200));
    // Helpers the provider left behind (MCP servers, updaters) end with the request.
    if let Some(job) = &job {
        job.terminate();
    }
    if cancel.is_cancelled() {
        return Err(CANCELLED.into());
    }
    if overflow.load(Ordering::SeqCst) {
        return Err("The provider printed more output than ArchGen accepts.".into());
    }
    Ok(Output {
        stdout: String::from_utf8(out).map_err(|_| "The provider's output is not UTF-8 text.")?,
        stderr: String::from_utf8_lossy(&err).into_owned(),
        success: status.success(),
    })
}

/// `text` cut to `limit` characters, for error messages that quote provider output.
pub fn truncate(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        text.into()
    } else {
        text.chars().take(limit).collect::<String>() + "…"
    }
}

static TEMP_SEQUENCE: AtomicU32 = AtomicU32::new(0);

/// A private folder under the system temp directory, removed with everything in it on drop.
/// Providers run inside one so they see no repository, no instructions file and nothing to edit.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!(
            "{TEMP_PREFIX}{}-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&path).map_err(|e| format!("creating a temporary folder: {e}"))?;
        Ok(Self(path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Writes `name` (a plain file name) inside the folder and returns its path.
    pub fn write(&self, name: &str, content: &str) -> Result<PathBuf, String> {
        let path = self.0.join(name);
        std::fs::write(&path, content).map_err(|e| format!("writing a temporary file: {e}"))?;
        Ok(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // A process that was just killed can hold its working folder for a moment.
        for _ in 0..10 {
            if std::fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Removes request folders a crashed or killed ArchGen left behind. `older_than` must
/// exceed the longest request so a second running instance never loses its folder.
pub fn sweep_stale_temp_dirs(older_than: Duration) {
    sweep_in(&std::env::temp_dir(), older_than);
}

fn sweep_in(root: &Path, older_than: Duration) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let stale = entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX)
            && entry.file_type().is_ok_and(|kind| kind.is_dir())
            && entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                .is_some_and(|age| age > older_than);
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn limits(seconds: u64) -> Limits {
        Limits::new(Duration::from_secs(seconds))
    }

    fn shell(script: &str) -> Command {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", script]);
        cmd
    }

    fn sleeper() -> Command {
        let mut cmd = Command::new("ping");
        cmd.args(["-n", "60", "127.0.0.1"]);
        cmd
    }

    #[test]
    fn stdin_reaches_the_child_and_output_comes_back() {
        let mut cmd = Command::new("findstr");
        cmd.arg("^");
        let input = "prvý riadok\nsecond line\n".repeat(20_000); // larger than a pipe buffer
        let output = run(cmd, &input, &limits(30), &Cancel::new()).unwrap();
        assert!(output.success);
        assert_eq!(output.stdout.replace("\r\n", "\n"), input);
    }

    #[test]
    fn failure_exit_is_reported_not_raised() {
        let output = run(shell("echo problem 1>&2 & exit /B 3"), "", &limits(30), &Cancel::new()).unwrap();
        assert!(!output.success);
        assert!(output.stderr.contains("problem"));
    }

    #[test]
    fn deadline_kills_the_process() {
        let started = Instant::now();
        let error = run(sleeper(), "", &limits(1), &Cancel::new()).unwrap_err();
        assert!(error.contains("did not answer within 1 s"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(8));
    }

    #[test]
    fn cancel_stops_a_running_process_and_blocks_a_later_one() {
        let cancel = Cancel::new();
        let remote = cancel.clone();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            remote.cancel();
        });
        let started = Instant::now();
        assert_eq!(run(sleeper(), "", &limits(60), &cancel).unwrap_err(), CANCELLED);
        assert!(started.elapsed() < Duration::from_secs(8));
        stopper.join().unwrap();
        assert_eq!(run(sleeper(), "", &limits(60), &cancel).unwrap_err(), CANCELLED);
    }

    // A descendant whose parent is gone keeps the pipes open; taskkill /T cannot find it.
    #[test]
    fn an_orphaned_descendant_delays_neither_the_answer_nor_the_deadline_nor_cancel() {
        let started = Instant::now();
        let output = run(shell("start /B ping -n 30 127.0.0.1 >NUL & echo parent-done"), "", &limits(20), &Cancel::new()).unwrap();
        assert!(output.stdout.contains("parent-done"));
        assert!(started.elapsed() < Duration::from_secs(8), "{:?}", started.elapsed());

        let started = Instant::now();
        let orphan = "start /B cmd /C start /B ping -n 30 127.0.0.1 & ping -n 30 127.0.0.1";
        assert!(run(shell(orphan), "", &limits(1), &Cancel::new()).unwrap_err().contains("did not answer"));
        assert!(started.elapsed() < Duration::from_secs(8), "{:?}", started.elapsed());

        let cancel = Cancel::new();
        let remote = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            remote.cancel();
        });
        let started = Instant::now();
        assert_eq!(run(shell(orphan), "", &limits(60), &cancel).unwrap_err(), CANCELLED);
        assert!(started.elapsed() < Duration::from_secs(8), "{:?}", started.elapsed());
    }

    #[test]
    fn oversized_stdout_is_refused_but_noisy_stderr_is_only_truncated() {
        let mut cmd = Command::new("findstr");
        cmd.arg("^");
        let small = Limits { timeout: Duration::from_secs(30), max_stdout: 1024, max_stderr: 1024 };
        let error = run(cmd, &"x".repeat(100_000), &small, &Cancel::new()).unwrap_err();
        assert!(error.contains("more output"), "{error}");

        // A streamed command, not a 400-iteration `cmd` built-in loop: `rust-test-quality#12`
        // found that loop 50-100x slower under CPU contention (each built-in is its own
        // scheduler round trip), occasionally stalling for tens of seconds against this test's
        // own 30 s deadline — a false failure unrelated to what the test checks. `findstr`
        // reads the noise from stdin in one streamed pass instead.
        let noise = "warning-line-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n".repeat(4_000); // 216 KB
        let output = run(shell("findstr . 1>&2 & echo answer"), &noise, &small, &Cancel::new()).unwrap();
        assert!(output.success && output.stdout.contains("answer"));
        assert_eq!(output.stderr.len(), 1024);
    }

    #[test]
    fn provider_processes_never_inherit_api_keys() {
        // Checked on the Command itself: the real environment is shared by every test thread.
        let mut cmd = shell("set");
        cmd.env("openai_api_key", "explicitly-passed").env("ARCHGEN_VISIBLE", "yes");
        scrub_keys(&mut cmd);
        let env: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(name, value)| (name.to_string_lossy().into_owned(), value.map(|v| v.to_string_lossy().into_owned())))
            .collect();
        for removed in ["ANTHROPIC_API_KEY", "GEMINI_API_KEY", "GOOGLE_API_KEY"] {
            assert!(env.contains(&(removed.to_string(), None)), "{removed} must be removed: {env:?}");
        }
        assert!(!env.iter().any(|(name, value)| name == "OPENAI_API_KEY" && value.is_none()));
        let output = run(cmd, "", &limits(30), &Cancel::new()).unwrap().stdout;
        assert!(output.contains("ARCHGEN_VISIBLE=yes") && output.to_lowercase().contains("openai_api_key=explicitly-passed"));
        assert!(output.contains("NoDefaultCurrentDirectoryInExePath=1"), "cmd.exe must not run programs from the working folder");
        assert!(!output.contains("ANTHROPIC_API_KEY=") && !output.contains("GOOGLE_API_KEY=") && !output.contains("GEMINI_API_KEY="));
    }

    #[test]
    fn temp_dirs_are_private_removed_and_swept_when_stale() {
        let dir = TempDir::new().unwrap();
        let file = dir.write("schema.json", "{}").unwrap();
        assert!(file.starts_with(dir.path()) && file.is_file());
        let path = dir.path().to_path_buf();
        drop(dir);
        assert!(!path.exists());

        let root = TempDir::new().unwrap();
        let stale = root.path().join(format!("{TEMP_PREFIX}stale"));
        let foreign = root.path().join("someone-elses-folder");
        std::fs::create_dir(&stale).unwrap();
        std::fs::create_dir(&foreign).unwrap();
        sweep_in(root.path(), Duration::from_secs(3600));
        assert!(stale.exists(), "a fresh folder belongs to a running request");
        std::thread::sleep(Duration::from_millis(30));
        sweep_in(root.path(), Duration::from_millis(10));
        assert!(!stale.exists() && foreign.exists());
    }

    #[test]
    fn locate_prefers_executables_and_truncate_counts_characters() {
        assert!(locate("cmd").is_some_and(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"))));
        assert!(locate("archgen-no-such-program").is_none());
        assert_eq!(truncate("  žltý kôň  ", 4), "žltý…");
        assert_eq!(truncate("ok", 4), "ok");
    }
}
