//! This module is used to control the lifetime of a running program.

use std::{
    ffi::c_int,
    fmt::Display,
    fs::{File, OpenOptions},
    os::unix::process::CommandExt,
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicU32, Ordering::Relaxed},
        Arc, Condvar, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use crate::{
    config::{ProgramConfig, RestartPolicy, StopSignal},
    LogEvent, LogEventKind, LogSender,
};

/// Opens a file for appending.
fn open_append(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

/// Creates a command from a program configuration.
///
/// The returned command can be invoked to start the program once.
fn create_command(config: &ProgramConfig) -> Command {
    let mut command = std::process::Command::new(&config.command);

    command.args(&config.args);
    command.env_clear();
    command.envs(&config.environment);

    if let Some(stdout) = &config.stdout {
        let file = open_append(&stdout).unwrap();
        command.stdout(file);
    } else {
        command.stdout(std::process::Stdio::null());
    }

    if let Some(stderr) = &config.stderr {
        let file = open_append(&stderr).unwrap();
        command.stderr(file);
    } else {
        command.stderr(std::process::Stdio::null());
    }

    if let Some(stdin) = &config.stdin {
        let file = std::fs::File::open(stdin).unwrap();
        command.stdin(file);
    } else {
        command.stdin(std::process::Stdio::null());
    }

    if let Some(dir) = &config.workdir {
        command.current_dir(dir);
    }

    if let Some(umask) = config.umask {
        unsafe {
            command.pre_exec(move || {
                libc::umask(umask);
                Ok(())
            });
        }
    }

    command
}

/// Sends a signal to a running process.
fn send_signal(pid: libc::pid_t, signal: StopSignal) -> Result<(), ProcessError> {
    let ret = unsafe { libc::kill(pid, signal.as_raw_signal()) };
    if ret != 0 {
        Err(ProcessError::NotStarted)
    } else {
        Ok(())
    }
}

/// An error that can occur when managing a running process.
#[derive(Debug)]
pub enum ProcessError {
    /// The process is already running and cannot be started again.
    AlreadyStarted,
    /// The process is not started.
    NotStarted,
    /// An unexpected I/O error occurred.
    Io(std::io::Error),
}

impl Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessError::AlreadyStarted => f.write_str("the process is already started"),
            ProcessError::NotStarted => f.write_str("the process is not started"),
            ProcessError::Io(err) => write!(f, "I/O error: {}", err),
        }
    }
}

impl From<std::io::Error> for ProcessError {
    #[inline]
    fn from(value: std::io::Error) -> Self {
        ProcessError::Io(value)
    }
}

/// Waits for a process to exit.
fn wait_pid(pid: libc::pid_t) -> std::io::Result<ExitCode> {
    let mut status = 0;
    let ret = unsafe { libc::waitpid(pid, &mut status, libc::WUNTRACED) };
    if ret == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(ExitCode(status))
    }
}

/// The exit code of a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitCode(pub c_int);

impl ExitCode {
    /// Returns the exit code that bash would have returned.
    pub fn like_bash(self) -> u32 {
        let status = self.0;

        if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status) as u32
        } else if libc::WIFSIGNALED(status) {
            128 + libc::WTERMSIG(status) as u32
        } else {
            status as u32
        }
    }
}

impl Display for ExitCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let st = self.0;

        if libc::WIFEXITED(st) {
            write!(f, "exited with code {}", libc::WEXITSTATUS(st))
        } else if libc::WIFSIGNALED(st) {
            let signal = match libc::WTERMSIG(st) {
                libc::SIGABRT => "SIGABRT",
                libc::SIGALRM => "SIGALRM",
                libc::SIGBUS => "SIGBUS",
                libc::SIGCHLD => "SIGCHLD",
                libc::SIGCONT => "SIGCONT",
                libc::SIGFPE => "SIGFPE",
                libc::SIGHUP => "SIGHUP",
                libc::SIGILL => "SIGILL",
                libc::SIGINT => "SIGINT",
                libc::SIGKILL => "SIGKILL",
                libc::SIGPIPE => "SIGPIPE",
                libc::SIGQUIT => "SIGQUIT",
                libc::SIGSEGV => "SIGSEGV",
                libc::SIGSTOP => "SIGSTOP",
                libc::SIGTERM => "SIGTERM",
                libc::SIGTSTP => "SIGTSTP",
                libc::SIGTTIN => "SIGTTIN",
                libc::SIGTTOU => "SIGTTOU",
                libc::SIGUSR1 => "SIGUSR1",
                libc::SIGUSR2 => "SIGUSR2",
                libc::SIGPROF => "SIGPROF",
                libc::SIGSYS => "SIGSYS",
                libc::SIGTRAP => "SIGTRAP",
                libc::SIGURG => "SIGURG",
                libc::SIGVTALRM => "SIGVTALRM",
                libc::SIGXCPU => "SIGXCPU",
                libc::SIGXFSZ => "SIGXFSZ",
                _ => "unknown",
            };

            write!(f, "terminated by signal {signal}")
        } else if libc::WIFSTOPPED(st) {
            write!(f, "stopped by signal {}", libc::WSTOPSIG(st))
        } else {
            write!(f, "unknown exit status: {}", st)
        }
    }
}

/// The state of the observer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ObserverState {
    /// The observer should exit.
    ExitingTaskmaster,
    /// The observer should spawn a new process.
    Spawn,
    /// The observer should not spawn a new process.
    Wait,
}

/// The name of a running process.
///
/// This includes its name in the configuration, as well as its replication index.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProcessName {
    /// The config name.
    pub name: Arc<str>,
    /// The replication index.
    pub index: usize,
}

impl Display for ProcessName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let buf = format!("{}-{}", self.name, self.index);
        f.pad(&buf)
    }
}

/// The state that is shared between the main thread and background threads.
struct ProcessState {
    /// The name of the process.
    name: ProcessName,
    /// The configuration of the process.
    config: ProgramConfig,

    /// Whether the process wants to be running.
    observer_state: Mutex<ObserverState>,
    /// A condition variable that is notified when the process wants to be running.
    observer_state_cond: Condvar,

    /// The PID of the process.
    pid: Mutex<Option<libc::pid_t>>,

    /// The number of times the process has been restarted.
    retry_count: AtomicU32,
}

impl ProcessState {
    /// Updates the `wants_to_be_running` flag.
    pub fn set_observer_state(&self, value: ObserverState) {
        let mut lock = self.observer_state.lock().unwrap();
        if *lock != value {
            *lock = value;
            self.observer_state_cond.notify_one();
        }
    }

    /// Waits until the observer state is no longer `Wait`. The new value is returned.
    pub fn wait_observer_state(&self) -> ObserverState {
        let mut state = self.observer_state.lock().unwrap();
        while *state == ObserverState::Wait {
            state = self.observer_state_cond.wait(state).unwrap();
        }
        *state
    }
}

/// Stores information about a running program.
pub struct Process {
    /// The shared state.
    state: Arc<ProcessState>,
    /// The observer thread that watches the process.
    observer_thread: JoinHandle<()>,
}

impl Process {
    /// Creates a new [`Process`] from its configuration.
    #[inline]
    pub fn new(log_sender: LogSender, name: ProcessName, config: ProgramConfig) -> Self {
        let start_now = config.at_launch;

        let state = Arc::new(ProcessState {
            name,
            config,

            observer_state: Mutex::new(if start_now {
                ObserverState::Spawn
            } else {
                ObserverState::Wait
            }),
            observer_state_cond: Condvar::new(),

            pid: Mutex::new(None),

            retry_count: AtomicU32::new(0),
        });

        let observer_thread = std::thread::spawn({
            let state = Arc::clone(&state);
            move || process_observer(log_sender, state)
        });

        Self {
            state,
            observer_thread,
        }
    }

    /// Returns the name of the process.
    #[inline]
    pub fn name(&self) -> &ProcessName {
        &self.state.name
    }

    /// Returns the configuration of the process.
    #[inline]
    pub fn config(&self) -> &ProgramConfig {
        &self.state.config
    }

    /// Requests the process to start.
    pub fn launch(&self) -> Result<(), ProcessError> {
        if self.state.pid.lock().unwrap().is_some() {
            return Err(ProcessError::AlreadyStarted);
        }

        self.state.set_observer_state(ObserverState::Spawn);
        Ok(())
    }

    /// Sends the provided signal to the process.
    ///
    /// This function requests the process to stop and prevents the observer thread
    /// from attempting to restart it.
    fn send_stop_signal(&self, signal: StopSignal) -> Result<(), ProcessError> {
        let pid = self.state.pid.lock().unwrap();
        let pid = pid.ok_or(ProcessError::NotStarted)?;
        send_signal(pid, signal)
    }

    /// Requests the process to stop.
    pub fn request_stop(&self) -> Result<(), ProcessError> {
        self.state.set_observer_state(ObserverState::Wait);
        self.send_stop_signal(self.state.config.signal)
    }

    /// Forces the process to stop.
    pub fn force_stop(&self) -> Result<(), ProcessError> {
        self.state.set_observer_state(ObserverState::Wait);
        self.send_stop_signal(StopSignal::Kill)
    }

    /// Requests the process to restart.
    pub fn request_restart(&self) -> Result<(), ProcessError> {
        self.state.retry_count.store(0, Relaxed);
        self.state.set_observer_state(ObserverState::Spawn);
        self.send_stop_signal(self.state.config.signal)
    }

    /// Forces the process to restart.
    pub fn force_restart(&self) -> Result<(), ProcessError> {
        self.state.retry_count.store(0, Relaxed);
        self.state.set_observer_state(ObserverState::Spawn);
        self.send_stop_signal(StopSignal::Kill)
    }
}

/// Observes a running process. This should be running in a background thread.
fn process_observer(log_sender: LogSender, state: Arc<ProcessState>) {
    let mut command = create_command(&state.config);

    let healthy_uptime = if state.config.healthy_uptime <= 0.0 {
        Duration::ZERO
    } else {
        Duration::from_secs_f64(state.config.healthy_uptime)
    };

    loop {
        // Wait until we need to do something.
        match state.wait_observer_state() {
            ObserverState::Spawn => (),
            ObserverState::ExitingTaskmaster => break,
            ObserverState::Wait => continue, // weird but ok
        }

        let pid = match command.spawn() {
            Ok(child) => child.id() as libc::pid_t,
            Err(err) => {
                log_sender
                    .send(LogEvent {
                        kind: LogEventKind::Failed(format!("Can't spawn child process: {err}")),
                        time: Instant::now(),
                        name: state.name.clone(),
                    })
                    .unwrap();
                state.set_observer_state(ObserverState::Wait);
                continue;
            }
        };

        assert!(state.pid.lock().unwrap().replace(pid).is_none());

        if healthy_uptime.is_zero() {
            log_sender
                .send(LogEvent {
                    kind: LogEventKind::Started,
                    time: Instant::now(),
                    name: state.name.clone(),
                })
                .unwrap();
        } else {
            log_sender
                .send(LogEvent {
                    kind: LogEventKind::Starting,
                    time: Instant::now(),
                    name: state.name.clone(),
                })
                .unwrap();

            let state = state.clone();
            let log_sender = log_sender.clone();
            std::thread::spawn(move || {
                std::thread::sleep(healthy_uptime);
                if state.pid.lock().unwrap().is_some() {
                    log_sender
                        .send(LogEvent {
                            kind: LogEventKind::Started,
                            time: Instant::now(),
                            name: state.name.clone(),
                        })
                        .unwrap();
                }
            });
        }

        let status = wait_pid(pid).unwrap();
        *state.pid.lock().unwrap() = None;

        log_sender
            .send(LogEvent {
                time: Instant::now(),
                name: state.name.clone(),
                kind: LogEventKind::Exited(status),
            })
            .unwrap();

        match state.config.restart {
            RestartPolicy::OnFailure if status.like_bash() == state.config.exit_code => {
                state.set_observer_state(ObserverState::Wait);
            }
            RestartPolicy::OnFailure | RestartPolicy::Always => {
                if state.retry_count.fetch_add(1, Relaxed) >= state.config.retries {
                    state.set_observer_state(ObserverState::Wait);
                }
            }
            RestartPolicy::Never => {
                state.set_observer_state(ObserverState::Wait);
            }
        }
    }
}
