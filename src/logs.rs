use std::{sync::Arc, time::Instant};

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
pub fn gather_logs(receiver: LogReceiver, taskmaster: Arc<Taskmaster>) {
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