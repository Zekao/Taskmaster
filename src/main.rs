use config::Config;
use logs::LogSender;
use program::{Process, ProcessName};

use std::sync::Arc;

mod commands;
mod config;
mod logs;
mod program;

fn main() {
    let config = Config::parse("config/run.yml".as_ref());
    let (log_sender, log_receiver) = std::sync::mpsc::channel();
    let taskmaster = Arc::new(Taskmaster::new(log_sender, config));

    std::thread::spawn({
        let taskmaster = taskmaster.clone();
        move || logs::gather_logs(log_receiver, taskmaster)
    });

    run_shell(taskmaster);
}

/// Contains the state of the program.
pub struct Taskmaster {
    processes: Vec<Process>,
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

        Self { processes }
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
