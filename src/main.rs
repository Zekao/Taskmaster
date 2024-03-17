use config::Config;
use logs::LogSender;
use program::{Process, ProcessName};

use std::{
    ffi::c_int,
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        Arc, RwLock,
    },
};

mod commands;
mod config;
mod logs;
mod program;

const CONFIG_DEFAULT_PATH: &str = "config/run.yml";
const LOG_DEFAULT_PATH: &str = "taskmaster.log";

fn main() -> std::process::ExitCode {
    let config = match Config::parse(CONFIG_DEFAULT_PATH.as_ref()) {
        Ok(ok) => ok,
        Err(err) => {
            eprintln!("\x1B[1;31merror\x1B[0m: can't parse config: {err}");
            return 2u8.into();
        }
    };

    let (log_sender, log_receiver) = std::sync::mpsc::channel();
    let taskmaster = Arc::new(RwLock::new(Taskmaster::new(log_sender, config)));

    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(LOG_DEFAULT_PATH)
        .unwrap();

    std::thread::spawn({
        let taskmaster = taskmaster.clone();
        move || logs::gather_logs(log_receiver, taskmaster, file)
    });

    run_shell(taskmaster);

    0u8.into()
}

/// Contains the state of the program.
pub struct Taskmaster {
    log_sender: LogSender,
    config: Config,
    processes: Vec<Process>,
}

impl Taskmaster {
    /// Creates a new [`Taskmaster`] instance.
    pub fn new(log_sender: LogSender, config: Config) -> Self {
        let mut processes = Vec::new();

        for (name, config) in config.programs.iter() {
            for replica_index in 0..config.replicas {
                let name = ProcessName {
                    name: Arc::from(name.as_str()),
                    index: replica_index,
                };

                processes.push(Process::new(log_sender.clone(), name, config.clone()));
            }
        }

        Self {
            log_sender,
            processes,
            config,
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

fn split_whitespace(s: &str) -> (&str, &str) {
    let index = s.find(char::is_whitespace).unwrap_or(s.len());
    s.split_at(index)
}

fn handle_commands(taskmaster: &RwLock<Taskmaster>, mut line: &str) {
    let command;
    (command, line) = split_whitespace(line);
    line = line.trim();

    match command.trim() {
        "start" => commands::start(line, &taskmaster.read().unwrap()),
        "stop" => commands::stop(line, &taskmaster.read().unwrap()),
        "restart" => commands::restart(line, &taskmaster.read().unwrap()),
        "status" => commands::status(line, &taskmaster.read().unwrap()),
        "reload" => commands::reload(line, &mut taskmaster.write().unwrap()),
        _ => println!("Unknown command: {}", command),
    }
}

static HANGED: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn hangup_handler(_signal: c_int) {
    HANGED.store(true, Relaxed);
}

fn check_hangup(taskmaster: Arc<RwLock<Taskmaster>>) {
    loop {
        if HANGED.swap(false, Relaxed) {
            println!("Hangup received, reloading config");
            commands::reload("", &mut taskmaster.write().unwrap());
        }
    }
}

/// Runs the shell.
fn run_shell(taskmaster: Arc<RwLock<Taskmaster>>) {
    unsafe {
        libc::signal(libc::SIGHUP, hangup_handler as usize);
    }

    std::thread::spawn({
        let taskmaster = taskmaster.clone();
        move || check_hangup(taskmaster)
    });

    let mut readline = ft::readline::Readline::new();

    while readline.read().unwrap() {
        readline.history_add_buffer().unwrap();
        println!();
        handle_commands(&taskmaster, readline.buffer());
    }
}
