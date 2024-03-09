use std::{os::unix::process::CommandExt, time::Instant};

use config::{Config, ProgramConfig, RestartPolicy};

mod config;

type LogSender = std::sync::mpsc::Sender<LogEvent>;
type LogReceiver = std::sync::mpsc::Receiver<LogEvent>;

fn main() {
    let config = Config::parse("config/run.yml".as_ref());

    let (log_sender, log_receiver) = std::sync::mpsc::channel();

    std::thread::spawn(move || gather_logs(log_receiver));

    for (name, config) in config.programs {
        if config.at_launch {
            run_program(&log_sender, &name, &config)
        }
    }

    run_shell();
}

/// Runs a program.
fn run_program(log_sender: &LogSender, name: &str, config: &ProgramConfig) {
    for i in 0..config.replicas {
        let log_sender = log_sender.clone();
        let name = name.to_owned();
        let config = config.clone();
        std::thread::spawn(move || run_program_instance(log_sender, name, i, config));
    }
}

/// The kind of a log event.
#[derive(Debug, Clone)]
pub enum LogEventKind {
    /// A process is starting.
    Starting,
    /// A process has started.
    Started,
    /// A process has failed to start.
    Failed(String),
    /// A process has exited.
    Exited {
        /// The status of the process.
        status: std::process::ExitStatus,
        /// Whether the status was expected or not.
        expected: bool,
    },
}

/// An event that can be logged.
#[derive(Debug, Clone)]
pub struct LogEvent {
    /// The kind of the event.
    pub kind: LogEventKind,
    /// The time of the event.
    pub time: Instant,
    /// The name of the entry that manages the process.
    pub name: String,
    /// The index of the process.
    pub index: usize,
}

/// Gathers the logs and do stuff with them.
fn gather_logs(receiver: LogReceiver) {
    let start_instant = Instant::now();

    while let Ok(ev) = receiver.recv() {
        let since_start = ev.time.saturating_duration_since(start_instant);

        let millis = since_start.subsec_millis();
        let secs = since_start.as_secs();
        let mins = secs / 60;
        let hours = mins / 60;
        print!(
            "{:02}:{:02}:{:02}.{:03}  ",
            hours,
            mins % 60,
            secs % 60,
            millis
        );

        print!("\x1B[1m{:<10}\x1B[0m  ", ev.name);

        match ev.kind {
            LogEventKind::Starting => print!("\x1B[1;36mSTARTING\x1B[0m  "),
            LogEventKind::Started => print!("\x1B[1;32mSTARTED\x1B[0m   "),
            LogEventKind::Failed(message) => print!("\x1B[1;31mFAILED\x1B[0m    {message}"),
            LogEventKind::Exited { status, expected } => {
                if expected {
                    print!("\x1B[1;33mEXITED\x1B[0m    ");
                } else {
                    print!("\x1B[1;31mFAILED\x1B[0m    ")
                }

                print!("{}", status);
            }
        }

        println!();
    }
}

/// Runs a single program instance.
fn run_program_instance(log_sender: LogSender, name: String, index: usize, config: ProgramConfig) {
    let mut command = std::process::Command::new(&config.command);

    command.args(&config.args);
    command.env_clear();
    command.envs(&config.environment);

    if let Some(stdout) = &config.stdout {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(stdout)
            .unwrap();
        command.stdout(file);
    } else {
        command.stdout(std::process::Stdio::null());
    }

    if let Some(stderr) = &config.stderr {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(stderr)
            .unwrap();
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

    let mut retries_count = 0;
    loop {
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) => {
                log_sender
                    .send(LogEvent {
                        kind: LogEventKind::Failed(format!("Can't spawn child process: {err}")),
                        time: Instant::now(),
                        name: name.clone(),
                        index,
                    })
                    .unwrap();
                return;
            }
        };

        if config.healthy_uptime != 0.0 {
            log_sender
                .send(LogEvent {
                    kind: LogEventKind::Starting,
                    time: Instant::now(),
                    name: name.clone(),
                    index,
                })
                .unwrap();
        } else {
            log_sender
                .send(LogEvent {
                    kind: LogEventKind::Started,
                    time: Instant::now(),
                    name: name.clone(),
                    index,
                })
                .unwrap();
        }

        let status = child.wait().unwrap();
        log_sender
            .send(LogEvent {
                time: Instant::now(),
                name: name.clone(),
                kind: LogEventKind::Exited {
                    status,
                    expected: status.code() == Some(config.exit_code),
                },
                index,
            })
            .unwrap();

        match config.restart {
            RestartPolicy::OnFailure => {
                if status.code() == Some(config.exit_code) {
                    break;
                }
            }
            RestartPolicy::Never => break,
            RestartPolicy::Always => (),
        }

        retries_count += 1;
        if retries_count >= config.retries {
            break;
        }
    }
}

/// Runs the shell.
fn run_shell() {
    let mut readline = ft::readline::Readline::new();

    while readline.read().unwrap() {
        readline.history_add_buffer().unwrap();
        println!("\n{:?}", readline.buffer());
    }
}
