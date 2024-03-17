use serde::Deserialize;
use std::{
    collections::BTreeMap,
    error::Error,
    path::{Path, PathBuf},
};

/// Deserializes a `umask` from a string as an octal number.
fn deserialize_umask<'de, D>(deserializer: D) -> Result<Option<libc::mode_t>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct UMaskVisitor;

    impl<'de> serde::de::Visitor<'de> for UMaskVisitor {
        type Value = libc::mode_t;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a number between 0 and 0o777")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            let value = libc::mode_t::from_str_radix(value, 8).map_err(E::custom)?;
            if value > 0o777 {
                return Err(E::custom("value must be between 0 and 0o777"));
            }
            Ok(value)
        }
    }

    deserializer.deserialize_str(UMaskVisitor).map(Some)
}

/// Indicates when to restart a process.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    /// Never restart the process.
    #[default]
    Never,
    /// Always restart the process.
    Always,
    /// Restart the process only if it fails (non-zero exit code).
    OnFailure,
}

/// The signal to send to a process to stop it.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum StopSignal {
    #[serde(rename = "SIGINT")]
    #[default]
    Int,
    #[serde(rename = "SIGTERM")]
    Term,
    #[serde(rename = "SIGHUP")]
    Hup,
    #[serde(rename = "SIGQUIT")]
    Quit,
    #[serde(rename = "SIGKILL")]
    Kill,
    #[serde(rename = "SIGUSR1")]
    Usr1,
    #[serde(rename = "SIGUSR2")]
    Usr2,
    #[serde(rename = "SIGSTOP")]
    Stop,
    #[serde(rename = "SIGALRM")]
    Alarm,
}

impl StopSignal {
    /// Returns the raw signal number for the signal.
    pub fn as_raw_signal(&self) -> libc::c_int {
        match self {
            StopSignal::Hup => libc::SIGHUP,
            StopSignal::Int => libc::SIGINT,
            StopSignal::Quit => libc::SIGQUIT,
            StopSignal::Term => libc::SIGTERM,
            StopSignal::Kill => libc::SIGKILL,
            StopSignal::Usr1 => libc::SIGUSR1,
            StopSignal::Usr2 => libc::SIGUSR2,
            StopSignal::Stop => libc::SIGSTOP,
            StopSignal::Alarm => libc::SIGALRM,
        }
    }
}

/// The configuration of a specific process.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProgramConfig {
    /// The command to use to start the program.
    pub command: PathBuf,
    /// The arguments to pass to the program.
    #[serde(default)]
    pub args: Vec<String>,
    /// The number of replicates to create.
    #[serde(default = "defaults::replicas")]
    pub replicas: usize,
    /// Whether to start the program at launch.
    #[serde(default = "defaults::at_launch")]
    pub at_launch: bool,
    /// The restart policy to use.
    #[serde(default)]
    pub restart: RestartPolicy,
    /// The expected exit code of the program.
    #[serde(default = "defaults::exit_code")]
    pub exit_code: u32,
    /// The amount of time to wait before marking the process as "healthy".
    #[serde(default = "defaults::healthy_uptime")]
    pub healthy_uptime: f64,
    /// The nu  mber of times to retry starting the process.
    #[serde(default = "defaults::retries")]
    pub retries: u32,
    /// The signal to send to the process to stop it.
    #[serde(default)]
    pub signal: StopSignal,
    /// The amount of time to wait before sending a `SIGKILL` signal to the process.
    #[serde(default = "defaults::exit_timeout")]
    pub exit_timeout: f64,
    /// If set, the process's standard output will be redirected to this file.
    #[serde(default)]
    pub stdout: Option<PathBuf>,
    /// If set, the process's standard error will be redirected to this file.
    #[serde(default)]
    pub stderr: Option<PathBuf>,
    /// If set, the process's standard input will be redirected from this file.
    #[serde(default)]
    pub stdin: Option<PathBuf>,
    /// The environment variables to set for the process.
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// The working directory to use for the process.
    #[serde(default)]
    pub workdir: Option<PathBuf>,
    /// The mask to apply when launching the process.
    #[serde(default, deserialize_with = "deserialize_umask")]
    pub umask: Option<libc::mode_t>,
}

mod defaults {
    pub fn retries() -> u32 {
        3
    }

    pub fn healthy_uptime() -> f64 {
        0.0
    }

    pub fn exit_code() -> u32 {
        0
    }

    pub fn at_launch() -> bool {
        false
    }

    pub fn replicas() -> usize {
        1
    }

    pub fn exit_timeout() -> f64 {
        10.0
    }
}

/// Contains the configuration of the file.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// The programs to start.
    pub programs: BTreeMap<String, ProgramConfig>,
}

impl Config {
    /// Parses the provided configuration file.
    ///
    /// # Panics
    ///
    /// This function panics if the file cannot be opened or parsed.
    pub fn parse(file: &Path) -> Result<Self, Box<dyn Error>> {
        let file = std::fs::File::open(file)?;
        let config = serde_yaml::from_reader(file)?;
        Ok(config)
    }

    /// Computes the difference between `old` and `self`.
    pub fn diff_since(&self, old: &Self) -> Vec<ConfigDiff> {
        let mut diffs = Vec::new();

        // Programs that are in `self`, but not in `old` are added.
        for (name, program) in self.programs.iter() {
            if !old.programs.contains_key(name) {
                diffs.push(ConfigDiff::AddedProgram(name.clone(), program.clone()));
            }
        }

        // Programs that are in `old`, but not in `self` are removed.
        for (name, _) in old.programs.iter() {
            if !self.programs.contains_key(name) {
                diffs.push(ConfigDiff::RemovedProgram(name.clone()));
            }
        }

        // Programs that are in both `old` and `self` are compared.
        for (name, program) in self.programs.iter() {
            if let Some(old_program) = old.programs.get(name) {
                if old_program != program {
                    diffs.push(ConfigDiff::ModifiedProgram(name.clone(), program.clone()));
                }
            }
        }

        diffs
    }
}

/// A difference between two [`Config`] instances.
pub enum ConfigDiff {
    /// A program has been added.
    AddedProgram(String, ProgramConfig),
    /// A program has been removed.
    RemovedProgram(String),
    /// A program has been modified.
    ModifiedProgram(String, ProgramConfig),
}
