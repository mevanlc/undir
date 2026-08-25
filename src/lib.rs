mod engine;
mod pathing;
mod platform;

use std::fmt;
use std::path::{Path, PathBuf};

use clap::ValueEnum;

pub use engine::run;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum OnError {
    #[default]
    Stop,
    Continue,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum Preflight {
    #[default]
    Fast,
    Full,
    Off,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CreateMode {
    #[default]
    Existing,
    One,
    Parents,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Options {
    pub srcdir: PathBuf,
    pub dstdir: PathBuf,
    pub merge: bool,
    pub overwrite: bool,
    pub on_error: OnError,
    pub keep: bool,
    pub preflight: Preflight,
    pub create: CreateMode,
    pub strict: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Setup,
    Preflight,
    Mutation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Issue {
    phase: Phase,
    source: Option<PathBuf>,
    destination: Option<PathBuf>,
    message: String,
}

impl Issue {
    pub(crate) fn setup(message: impl Into<String>) -> Self {
        Self::new(Phase::Setup, None, None, message)
    }

    pub(crate) fn at(
        phase: Phase,
        source: Option<&Path>,
        destination: Option<&Path>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(
            phase,
            source.map(Path::to_path_buf),
            destination.map(Path::to_path_buf),
            message,
        )
    }

    fn new(
        phase: Phase,
        source: Option<PathBuf>,
        destination: Option<PathBuf>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            phase,
            source,
            destination,
            message: message.into(),
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }

    pub fn destination(&self) -> Option<&Path> {
        self.destination.as_deref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let phase = match self.phase {
            Phase::Setup => "setup",
            Phase::Preflight => "preflight",
            Phase::Mutation => "mutation",
        };
        write!(formatter, "{phase}: ")?;
        match (&self.source, &self.destination) {
            (Some(source), Some(destination)) => {
                write!(formatter, "{source:?} -> {destination:?}: ")?;
            }
            (Some(source), None) => write!(formatter, "{source:?}: ")?,
            (None, Some(destination)) => write!(formatter, "{destination:?}: ")?,
            (None, None) => {}
        }
        formatter.write_str(&self.message)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Report {
    issues: Vec<Issue>,
}

impl Report {
    pub(crate) fn from_issue(issue: Issue) -> Self {
        Self {
            issues: vec![issue],
        }
    }

    pub(crate) fn from_issues(issues: Vec<Issue>) -> Self {
        debug_assert!(!issues.is_empty());
        Self { issues }
    }

    pub fn issues(&self) -> &[Issue] {
        &self.issues
    }
}
