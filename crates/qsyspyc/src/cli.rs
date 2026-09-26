use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "qsyspyc",
    about = "QSysPy Compiler - Python to Q-SYS Lua Transpiler",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Build the Python input code and emit a Q-SYS artifact (.qplug or .lua)
    Build {
        /// Path to the input Python source file
        input: PathBuf,

        /// Custom output path for the emitted artifact
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Minify the output code
        #[arg(long, default_value_t = false)]
        minify: bool,
    },

    /// Check the validity of the Python source file without emitting code
    Check {
        /// Path to the input Python source file
        input: PathBuf,
    },

    /// Generate PEP 561 Python type stubs (.pyi) for IDE autocompletion (VS Code, PyCharm, Pyright)
    GenStubs {
        /// Target directory for type stubs (e.g. 'typings' or '.')
        #[arg(default_value = "typings")]
        out_dir: PathBuf,
    },

    /// Start the Language Server Protocol (LSP) daemon on stdio
    Lsp,
}
