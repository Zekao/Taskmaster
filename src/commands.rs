use crate::Taskmaster;

pub fn status(line: &str, taskmaster: &Taskmaster) {
    let _ = (line, taskmaster);
    todo!("status")
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
