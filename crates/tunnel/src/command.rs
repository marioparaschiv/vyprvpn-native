use std::fmt;
use std::io;
use std::path::PathBuf;
use std::process::{Command, ExitStatus};

/// One command to run, with its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Invocation {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl Invocation {
    pub(crate) fn new(program: impl Into<PathBuf>, args: &[&str]) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(|&arg| arg.to_owned()).collect(),
        }
    }
}

impl fmt::Display for Invocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.program.display(), self.args.join(" "))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("running {command}")]
    Spawn {
        command: String,
        #[source]
        source: io::Error,
    },
    #[error("{command} failed ({status}): {output}")]
    Failed {
        command: String,
        status: ExitStatus,
        output: String,
    },
}

/// Runs `invocation` and returns its stdout, or its stdout and stderr as an error when it exits
/// non-zero.
pub(crate) fn run(invocation: &Invocation) -> Result<String, CommandError> {
    let output = Command::new(&invocation.program)
        .args(&invocation.args)
        .output()
        .map_err(|source| CommandError::Spawn {
            command: invocation.to_string(),
            source,
        })?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();

    if output.status.success() {
        return Ok(text);
    }

    // netsh and networksetup report most errors on stdout.
    text.push_str(&String::from_utf8_lossy(&output.stderr));

    Err(CommandError::Failed {
        command: invocation.to_string(),
        status: output.status,
        output: text.trim().to_owned(),
    })
}
