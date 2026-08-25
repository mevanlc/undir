use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, Parser, ValueEnum};
use undir::{CreateMode, OnError, Options, Preflight};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CompletionShell {
    Bash,
    Elvish,
    Fish,
    Pwsh,
    Zsh,
}

impl From<CompletionShell> for clap_complete::Shell {
    fn from(shell: CompletionShell) -> Self {
        match shell {
            CompletionShell::Bash => Self::Bash,
            CompletionShell::Elvish => Self::Elvish,
            CompletionShell::Fish => Self::Fish,
            CompletionShell::Pwsh => Self::PowerShell,
            CompletionShell::Zsh => Self::Zsh,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    version,
    about,
    override_usage = "undir [OPTIONS] <SRCDIR> [DSTDIR]\n       undir --completion <SHELL>"
)]
struct Cli {
    /// Directory whose children will be moved.
    #[arg(required_unless_present = "completion")]
    srcdir: Option<PathBuf>,

    /// Directory into which the children will be moved.
    #[arg(default_value = ".")]
    dstdir: PathBuf,

    /// Merge source directories into existing destination directories.
    #[arg(long)]
    merge: bool,

    /// Replace destination nondirectories with source nondirectories.
    #[arg(long)]
    overwrite: bool,

    /// What to do after a mutation-time filesystem error.
    #[arg(long, value_enum, default_value_t)]
    error: OnError,

    /// Keep srcdir after all of its children have been moved.
    #[arg(long)]
    keep: bool,

    /// How thoroughly to check the operation before mutation.
    #[arg(long, value_enum, default_value_t)]
    preflight: Preflight,

    /// Create dstdir, requiring its parent to exist and dstdir not to exist.
    #[arg(long, conflicts_with = "mkdirs")]
    mkdir: bool,

    /// Create dstdir and any missing parents, like mkdir --parents.
    #[arg(long)]
    mkdirs: bool,

    /// Require atomic no-clobber rename support for missing destinations.
    #[arg(long)]
    strict: bool,

    /// Generate a completion script for a shell.
    #[arg(long, value_enum, value_name = "SHELL", exclusive = true)]
    completion: Option<CompletionShell>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Some(shell) = cli.completion {
        return write_completion(shell);
    }

    let create = if cli.mkdir {
        CreateMode::One
    } else if cli.mkdirs {
        CreateMode::Parents
    } else {
        CreateMode::Existing
    };
    let options = Options {
        srcdir: cli
            .srcdir
            .expect("clap requires srcdir unless --completion is present"),
        dstdir: cli.dstdir,
        merge: cli.merge,
        overwrite: cli.overwrite,
        on_error: cli.error,
        keep: cli.keep,
        preflight: cli.preflight,
        create,
        strict: cli.strict,
    };

    match undir::run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(report) => {
            for issue in report.issues() {
                eprintln!("undir: {issue}");
            }
            ExitCode::FAILURE
        }
    }
}

fn write_completion(shell: CompletionShell) -> ExitCode {
    let mut command = Cli::command();
    let binary_name = command.get_name().to_owned();
    let generator: clap_complete::Shell = shell.into();
    let mut script = Vec::new();
    clap_complete::generate(generator, &mut command, binary_name, &mut script);

    match io::stdout().write_all(&script) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("undir: failed to write completion script: {error}");
            ExitCode::FAILURE
        }
    }
}
