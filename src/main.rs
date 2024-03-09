use config::{Config, ProgramConfig};

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
    /// A process has started.
    Started,
    /// A process has failed to start.
    Failed(String),
    /// A process has exited.
    Exited,
}

impl std::fmt::Display for LogEventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            LogEventKind::Started => f.pad("STARTED"),
            LogEventKind::Failed(_) => f.pad("FAILED"),
            LogEventKind::Exited => f.pad("EXITED"),
        }
    }
}

/// An event that can be logged.
#[derive(Debug, Clone)]
pub struct LogEvent {
    /// The kind of the event.
    pub kind: LogEventKind,
    /// The time of the event.
    pub time: ft::Instant,
    /// The name of the entry that manages the process.
    pub name: String,
}

/// Gathers the logs and do stuff with them.
fn gather_logs(receiver: LogReceiver) {
    let start_instant = ft::Clock::MONOTONIC.get();

    while let Ok(ev) = receiver.recv() {
        let since_start = ev.time.saturating_sub(start_instant);

        print!("{:<10}  ", format!("{:#?}", since_start));
        print!("\x1B[1m{:<10}\x1B[0m", ev.name);

        match ev.kind {
            LogEventKind::Started => print!("\x1B[1;32m{:<10}\x1B[0m", ev.kind),
            LogEventKind::Failed(_) => print!("\x1B[1;31m{:<10}\x1B[0m {message}", ev.kind),
            LogEventKind::Exited => print!("\x1B[1;35m{:<10}\x1B[0m", ev.kind),
        }

        println!();
    }
}

/// Runs a single program instance.
fn run_program_instance(log_sender: LogSender, name: String, index: usize, config: ProgramConfig) {
    let result = std::process::Command::new(&config.command)
        .args(&config.args)
        .env_clear()
        .envs(&config.environment)
        .spawn();

    let child = match result {
        Ok(child) => child,
        Err(err) => {
            log_sender
                .send(LogEvent {
                    kind: LogEventKind::Failed(format!("Can't spawn child process: {err}")),
                    time: ft::Clock::MONOTONIC.get(),
                    name,
                })
                .unwrap();
            return;
        }
    };

    log_sender
        .send(LogEvent {
            kind: LogEventKind::Started,
            time: ft::Clock::MONOTONIC.get(),
            name,
        })
        .unwrap();
}

/// Runs the shell.
fn run_shell() {
    let mut readline = ft::readline::Readline::new();

    while readline.read().unwrap() {
        readline.history_add_buffer().unwrap();
        println!("\n{:?}", readline.buffer());
    }
}
