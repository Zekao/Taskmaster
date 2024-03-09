use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Deserializes a `umask` from a string as an octal number.
fn deserialize_umask<'de, D>(deserializer: D) -> Result<Option<ft::fd::Mode>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct UMaskVisitor;

    impl<'de> serde::de::Visitor<'de> for UMaskVisitor {
        type Value = ft::fd::Mode;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a number between 0 and 0o777")
        }

        fn visit_str<E>(self, value: &str) -> Result<ft::fd::Mode, E>
        where
            E: serde::de::Error,
        {
            let value = u16::from_str_radix(value, 8).map_err(E::custom)?;
            if value > 0o777 {
                return Err(E::custom("value must be between 0 and 0o777"));
            }
            Ok(ft::fd::Mode::from_bits_retain(value))
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
}

/// The configuration of a specific process.
#[derive(Debug, Clone, Deserialize)]
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
    pub exit_code: i32,
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
    #[serde(default)]
    pub exit_timeout: Option<f64>,
    /// If set, the process's standard output will be redirected to this file.
    #[serde(default = "defaults::stdout")]
    pub stdout: PathBuf,
    /// If set, the process's standard error will be redirected to this file.
    #[serde(default = "defaults::stderr")]
    pub stderr: PathBuf,
    /// The environment variables to set for the process.
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// The working directory to use for the process.
    #[serde(default)]
    pub workdir: Option<PathBuf>,
    /// The mask to apply when launching the process.
    #[serde(default, deserialize_with = "deserialize_umask")]
    pub umask: Option<ft::fd::Mode>,
}

mod defaults {
    use std::path::PathBuf;

    pub fn stdout() -> PathBuf {
        PathBuf::from("/dev/null")
    }

    pub fn stderr() -> PathBuf {
        PathBuf::from("/dev/null")
    }

    pub fn retries() -> u32 {
        3
    }

    pub fn healthy_uptime() -> f64 {
        0.0
    }

    pub fn exit_code() -> i32 {
        0
    }

    pub fn at_launch() -> bool {
        false
    }

    pub fn replicas() -> usize {
        1
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
    pub fn parse(file: &Path) -> Self {
        let file = std::fs::File::open(file).unwrap();
        serde_yaml::from_reader(file).unwrap()
    }
}
