use std::sync::Arc;

use crate::{
    config::{Config, ConfigDiff},
    program::{Process, ProcessName},
    Taskmaster, CONFIG_DEFAULT_PATH,
};

pub fn status(line: &str, taskmaster: &Taskmaster) {
    let _ = (line, taskmaster);
    for process in taskmaster.processes.iter() {
        match process.state.pid.lock().unwrap().as_ref() {
            Some(content) => {
                println!("{:<12} | {:<6} | running", process.name().name, content.pid)
            }
            None => {
                println!("{:<12} | {:6} | not running", process.name().name, "");
            }
        }
    }
}

pub fn start(line: &str, taskmaster: &Taskmaster) {
    if taskmaster.get_processes_by_name(line).next().is_none() {
        println!("Process not found");
        return;
    }
    for process in taskmaster.get_processes_by_name(line) {
        if let Err(err) = process.launch() {
            println!("Error: {}", err);
        }
    }
}

pub fn stop(line: &str, taskmaster: &Taskmaster) {
    if taskmaster.get_processes_by_name(line).next().is_none() {
        println!("Process not found");
        return;
    }
    for process in taskmaster.get_processes_by_name(line) {
        if let Err(err) = process.request_stop() {
            println!("Error: {}", err);
        }
    }
}

pub fn restart(line: &str, taskmaster: &Taskmaster) {
    if taskmaster.get_processes_by_name(line).next().is_none() {
        println!("Process not found");
        return;
    }
    for process in taskmaster.get_processes_by_name(line) {
        if let Err(err) = process.request_restart() {
            println!("Error: {}", err);
        }
    }
}

pub fn reload(_line: &str, taskmaster: &mut Taskmaster) {
    let new_config = match Config::parse(CONFIG_DEFAULT_PATH.as_ref()) {
        Ok(config) => config,
        Err(err) => {
            println!("\x1B[1;31merror\x1B[0m: can't reload config: {err}");
            return;
        }
    };
    let diff = new_config.diff_since(&taskmaster.config);

    if diff.is_empty() {
        println!("No changes");
        return;
    }

    for diff in diff {
        match diff {
            ConfigDiff::AddedProgram(name, config) => {
                println!("adding `{name}`");

                for replica_index in 0..config.replicas {
                    let name = ProcessName {
                        name: Arc::from(name.as_str()),
                        index: replica_index,
                    };

                    println!("adding replica `{name}`");
                    taskmaster.processes.push(Process::new(
                        taskmaster.log_sender.clone(),
                        name.clone(),
                        config.clone(),
                    ));

                    if config.at_launch {
                        let _ = taskmaster
                            .get_process_by_process_name(&name)
                            .unwrap()
                            .launch();
                    }
                }
            }
            ConfigDiff::ModifiedProgram(name, config) => {
                println!("reloading `{name}`");

                taskmaster
                    .processes
                    .retain(|p| p.name().name.as_ref() == name.as_str());

                for index in 0..config.replicas {
                    let name = ProcessName {
                        name: Arc::from(name.as_str()),
                        index,
                    };
                    taskmaster.processes.push(Process::new(
                        taskmaster.log_sender.clone(),
                        name.clone(),
                        config.clone(),
                    ));

                    if config.at_launch {
                        let _ = taskmaster
                            .get_process_by_process_name(&name)
                            .unwrap()
                            .launch();
                    }
                }
            }
            ConfigDiff::RemovedProgram(name) => {
                println!("removing `{name}`");

                taskmaster
                    .processes
                    .retain(|p| p.name().name.as_ref() == name.as_str());
            }
        }
    }

    taskmaster.config = new_config;
}
