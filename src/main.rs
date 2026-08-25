use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use undir::{CreateMode, OnError, Options, Preflight};

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Directory whose children will be moved.
    srcdir: PathBuf,

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
    on_error: OnError,

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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let create = if cli.mkdir {
        CreateMode::One
    } else if cli.mkdirs {
        CreateMode::Parents
    } else {
        CreateMode::Existing
    };
    let options = Options {
        srcdir: cli.srcdir,
        dstdir: cli.dstdir,
        merge: cli.merge,
        overwrite: cli.overwrite,
        on_error: cli.on_error,
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
