//! This module is used to control the lifetime of a running program.

use std::{
    ffi::c_int,
    fmt::Display,
    fs::{File, OpenOptions},
    os::unix::process::CommandExt,
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};

use libc::pid_t;

use crate::{
    config::{ProgramConfig, RestartPolicy, StopSignal},
    logs::{LogEvent, LogEventKind},
    LogSender,
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

/// The state of the observer thread.
#[derive(Debug)]
pub struct ObserverState {
    /// Whether the process structure itself is being removed.
    pub process_removed: bool,
    /// Whether new processes should spawned when possible.
    pub standby: bool,
    /// Whether the process must be restarted regardless of its exit code.
    ///
    /// This also resets the restart count.
    pub restart: bool,
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

/// Information about the process that is currently running.
struct RunningProcess {
    pub started_at: Instant,
    pub pid: pid_t,
}

impl RunningProcess {
    pub fn started_right_now(pid: pid_t) -> Self {
        Self {
            started_at: Instant::now(),
            pid,
        }
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
    pid: Mutex<Option<RunningProcess>>,
}

impl ProcessState {
    /// Updates the observer state and notifies the condition variable.
    pub fn update_observer_state(&self, f: impl FnOnce(&mut ObserverState)) {
        let mut state = self.observer_state.lock().unwrap();
        f(&mut state);
        self.observer_state_cond.notify_all();
    }

    pub fn send_stop_signal(&self, signal: StopSignal) -> Result<(), ProcessError> {
        let running_process = self.pid.lock().unwrap();
        let running_process = running_process.as_ref().ok_or(ProcessError::NotStarted)?;
        send_signal(running_process.pid, signal)
    }

    pub fn force_stop(&self) -> Result<(), ProcessError> {
        self.update_observer_state(|s| s.standby = true);
        self.send_stop_signal(StopSignal::Kill)
    }
}

pub struct Process {
    state: Arc<ProcessState>,
    log_sender: LogSender,
}

impl Process {
    /// Creates a new [`Process`] from its configuration.
    #[inline]
    pub fn new(log_sender: LogSender, name: ProcessName, config: ProgramConfig) -> Self {
        let start_now = config.at_launch;

        let state = Arc::new(ProcessState {
            name,
            config,

            observer_state: Mutex::new(ObserverState {
                process_removed: false,
                standby: !start_now,
                restart: false,
            }),
            observer_state_cond: Condvar::new(),

            pid: Mutex::new(None),
        });

        std::thread::spawn({
            let state = Arc::clone(&state);
            let log_sender = log_sender.clone();
            move || process_observer(log_sender, state)
        });

        Self { state, log_sender }
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

        self.state.update_observer_state(|s| s.standby = false);
        Ok(())
    }

    /// Requests the process to stop.
    ///
    /// This function will also create a new thread in order to check if the process has been
    /// correctly stopped after the configured timeout.
    pub fn request_stop(&self) -> Result<(), ProcessError> {
        let state = self.state.clone();
        let log_sender = self.log_sender.clone();

        state.send_stop_signal(self.state.config.signal)?;
        state.update_observer_state(|s| s.standby = true);

        std::thread::spawn(move || {
            // The process is still running because the STARTED instant is still in the past.
            // ---------------------------------------------->
            //          |                |
            //        STARTED          STOP REQUEST
            //
            // The process has stoped because the STARTED instant is in the future.
            // ---------------------------------------------->
            //         |                |
            //      STOP REQUEST      STARTED

            let stop_request_instant = Instant::now();

            std::thread::sleep(duration_from_f64(state.config.exit_timeout));

            let running_process = state.pid.lock().unwrap();
            if running_process
                .as_ref()
                .is_some_and(|running_process| running_process.started_at < stop_request_instant)
            {
                drop(running_process);

                if let Err(err) = state.force_stop() {
                    println!("failed to force_stop: {}", err);
                }
                log_sender
                    .send(LogEvent {
                        kind: LogEventKind::Killed,
                        time: Instant::now(),
                        name: state.name.clone(),
                    })
                    .unwrap();
            }
        });

        Ok(())
    }

    /// Forces the process to stop.
    pub fn force_stop(&self) -> Result<(), ProcessError> {
        self.state.force_stop()
    }

    /// Requests the process to restart.
    pub fn request_restart(&self) -> Result<(), ProcessError> {
        self.state.update_observer_state(|s| {
            s.standby = true;
            s.restart = true;
        });
        self.state.send_stop_signal(self.state.config.signal)
    }

    /// Forces the process to restart.
    pub fn force_restart(&self) -> Result<(), ProcessError> {
        self.state.update_observer_state(|s| {
            s.standby = true;
            s.restart = true;
        });
        self.state.send_stop_signal(StopSignal::Kill)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.force_stop().unwrap();
        self.state
            .update_observer_state(|s| s.process_removed = true);
    }
}

/// Observes a running process. This should be running in a background thread.
fn process_observer(log_sender: LogSender, state: Arc<ProcessState>) {
    let mut command = create_command(&state.config);

    let healthy_uptime = duration_from_f64(state.config.healthy_uptime);

    let mut retry_count = 0;

    'main: loop {
        // Wait until we need to do something.
        {
            let mut lock = state.observer_state.lock().unwrap();
            while lock.standby && !lock.restart {
                if lock.process_removed {
                    break 'main;
                }

                lock = state.observer_state_cond.wait(lock).unwrap();
            }

            if lock.restart {
                lock.restart = false;
                retry_count = 0;
            }
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
                state.update_observer_state(|s| s.standby = true);
                continue;
            }
        };

        *state.pid.lock().unwrap() = Some(RunningProcess::started_right_now(pid));

        let has_been_stopped = Arc::new(AtomicBool::new(false));

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

            let name = state.name.clone();
            let log_sender = log_sender.clone();
            let has_been_stopped = has_been_stopped.clone();
            std::thread::spawn(move || {
                std::thread::sleep(healthy_uptime);
                if !has_been_stopped.load(Relaxed) {
                    log_sender
                        .send(LogEvent {
                            kind: LogEventKind::Started,
                            time: Instant::now(),
                            name,
                        })
                        .unwrap();
                }
            });
        }

        let status = wait_pid(pid).unwrap();
        *state.pid.lock().unwrap() = None;
        has_been_stopped.store(true, Relaxed);

        log_sender
            .send(LogEvent {
                time: Instant::now(),
                name: state.name.clone(),
                kind: LogEventKind::Exited(status),
            })
            .unwrap();

        match state.config.restart {
            RestartPolicy::OnFailure if status.like_bash() == state.config.exit_code => {
                state.observer_state.lock().unwrap().standby = true;
            }
            RestartPolicy::OnFailure | RestartPolicy::Always => {
                retry_count += 1;
                if retry_count > state.config.retries {
                    state.observer_state.lock().unwrap().standby = true;
                }
            }
            RestartPolicy::Never => {
                state.observer_state.lock().unwrap().standby = true;
            }
        }
    }
}

fn duration_from_f64(value: f64) -> Duration {
    Duration::try_from_secs_f64(value).unwrap_or_default()
}
