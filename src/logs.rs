use std::{
    io::Write,
    sync::{Arc, RwLock},
    time::Instant,
};

use crate::{
    program::{ExitCode, ProcessName},
    Taskmaster,
};

pub type LogSender = std::sync::mpsc::Sender<LogEvent>;
pub type LogReceiver = std::sync::mpsc::Receiver<LogEvent>;

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
    /// A process has been killed.
    Killed,
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
pub fn gather_logs(
    receiver: LogReceiver,
    taskmaster: Arc<RwLock<Taskmaster>>,
    mut file: std::fs::File,
) {
    let start_instant = Instant::now();
    let mut client = reqwest::blocking::Client::new();
    while let Ok(ev) = receiver.recv() {
        let since_start = ev.time.saturating_duration_since(start_instant);

        let millis = since_start.subsec_millis();
        let secs = since_start.as_secs();
        let mins = secs / 60;
        let hours = mins / 60;
        special_print(
            &format!(
                "{:02}:{:02}:{:02}.{:03}  ",
                hours,
                mins % 60,
                secs % 60,
                millis
            ),
            &mut file,
            &mut client,
        );

        special_print(&format!("{: <10}  ", ev.name), &mut file, &mut client);

        match ev.kind {
            LogEventKind::Starting => {
                special_print("\x1B[1;36mSTARTING\x1B[0m  ", &mut file, &mut client)
            }
            LogEventKind::Started => {
                special_print("\x1B[1;32mSTARTED\x1B[0m   ", &mut file, &mut client)
            }
            LogEventKind::Failed(message) => {
                special_print("\x1B[1;31mFAILED\x1B[0m    ", &mut file, &mut client);
                special_print(&message, &mut file, &mut client);
            }
            LogEventKind::Exited(status) => {
                if taskmaster
                    .read()
                    .unwrap()
                    .get_process_by_process_name(&ev.name)
                    .is_some_and(|p| {
                        !p.config()
                            .read()
                            .unwrap()
                            .exit_code
                            .contains(&status.like_bash())
                    })
                {
                    special_print("\x1B[1;31mFAILED\x1B[0m    ", &mut file, &mut client);
                } else {
                    special_print("\x1B[1;33mEXITED\x1B[0m    ", &mut file, &mut client);
                }

                special_print(&format!("exit code {}", status), &mut file, &mut client);
            }
            LogEventKind::Killed => {
                special_print("\x1B[1;31mKILLED\x1B[0m    ", &mut file, &mut client);
            }
        }

        special_print("\n", &mut file, &mut client);
    }
}

/// Prints a string to the standard output and to a file.
fn special_print(string: &str, file: &mut std::fs::File, client: &mut reqwest::blocking::Client) {
    let _ = client
        .post(std::env::var("SERVER_URL").unwrap_or("http://localhost:8080/".into()))
        .body(string.to_owned())
        .send();
    print!("{}", string);
    file.write_all(string.as_bytes()).unwrap();
}
