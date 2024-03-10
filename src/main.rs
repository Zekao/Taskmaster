use config::Config;
use program::{ExitCode, Process, ProcessName};
use std::{sync::Arc, time::Instant};

mod commands;
mod config;
mod program;

type LogSender = std::sync::mpsc::Sender<LogEvent>;
type LogReceiver = std::sync::mpsc::Receiver<LogEvent>;

fn main() {
    let config = Config::parse("config/run.yml".as_ref());
    let (log_sender, log_receiver) = std::sync::mpsc::channel();
    let taskmaster = Arc::new(Taskmaster::new(log_sender, config));

    std::thread::spawn({
        let taskmaster = taskmaster.clone();
        move || gather_logs(log_receiver, taskmaster)
    });

    run_shell(taskmaster);
}

/// Contains the state of the program.
pub struct Taskmaster {
    processes: Vec<Process>,
    log_sender: LogSender,
}

impl Taskmaster {
    /// Creates a new [`Taskmaster`] instance.
    pub fn new(log_sender: LogSender, config: Config) -> Self {
        let mut processes = Vec::new();

        for (name, config) in config.programs {
            for replica_index in 0..config.replicas {
                let name = ProcessName {
                    name: Arc::from(name.as_str()),
                    index: replica_index,
                };

                processes.push(Process::new(log_sender.clone(), name, config.clone()));
            }
        }

        Self {
            processes,
            log_sender,
        }
    }

    /// Gets a process by its name.
    #[inline]
    pub fn get_process_by_process_name(&self, name: &ProcessName) -> Option<&Process> {
        self.processes.iter().find(|p| p.name() == name)
    }

    /// Returns an iterator over all the processes.
    pub fn get_processes_by_name<'a>(
        &'a self,
        name: &'a str,
    ) -> impl 'a + Iterator<Item = &'a Process> {
        self.processes
            .iter()
            .filter(move |p| p.name().name.as_ref() == name)
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
    Exited(ExitCode),
}

/// An event that can be logged.
#[derive(Debug, Clone)]
pub struct LogEvent {
    /// The kind of the event.
    pub kind: LogEventKind,
    /// The time of the event.
    pub time: Instant,
    /// The name of the process for which this event is.
    pub name: ProcessName,
}

/// Gathers the logs and do stuff with them.
fn gather_logs(receiver: LogReceiver, taskmaster: Arc<Taskmaster>) {
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

        print!("\x1B[1m{: <10}\x1B[0m  ", ev.name);

        match ev.kind {
            LogEventKind::Starting => print!("\x1B[1;36mSTARTING\x1B[0m  "),
            LogEventKind::Started => print!("\x1B[1;32mSTARTED\x1B[0m   "),
            LogEventKind::Failed(message) => print!("\x1B[1;31mFAILED\x1B[0m    {message}"),
            LogEventKind::Exited(status) => {
                if taskmaster
                    .get_process_by_process_name(&ev.name)
                    .is_some_and(|p| p.config().exit_code != status.like_bash())
                {
                    print!("\x1B[1;31mFAILED\x1B[0m    ")
                } else {
                    print!("\x1B[1;33mEXITED\x1B[0m    ");
                }

                print!("exit code {}", status);
            }
        }

        println!();
    }
}

fn split_whitespace(s: &str) -> (&str, &str) {
    let index = s.find(char::is_whitespace).unwrap_or(s.len());
    s.split_at(index)
}

fn handle_commands(taskmaster: &Taskmaster, mut line: &str) {
    let command;
    (command, line) = split_whitespace(line);
    line = line.trim();

    match command.trim() {
        "start" => commands::start(line, taskmaster),
        "stop" => commands::stop(line, taskmaster),
        "restart" => commands::restart(line, taskmaster),
        "status" => commands::status(line, taskmaster),
        _ => println!("Unknown command: {}", command),
    }
}
/// Runs the shell.
fn run_shell(taskmaster: Arc<Taskmaster>) {
    let mut readline = ft::readline::Readline::new();

    while readline.read().unwrap() {
        readline.history_add_buffer().unwrap();
        println!();
        handle_commands(&taskmaster, readline.buffer());
    }
}
