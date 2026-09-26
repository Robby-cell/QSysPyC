mod cli;
mod lsp;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use cli::{Cli, Commands};

use qsys_compiler::{Compiler, CompilerOptions};
use qsys_diagnostics::{Diagnostics, SourceFile};

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Commands::Build {
            input,
            output,
            minify,
        } => run_build(&input, output.as_deref(), minify),
        Commands::Check { input } => run_check(&input),
        Commands::GenStubs { out_dir } => run_gen_stubs(&out_dir),
        Commands::Lsp => {
            // Launch async Tokio runtime for the LSP stdio server
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Failed to start Tokio runtime");
            rt.block_on(lsp::run_server());
            ExitCode::SUCCESS
        }
    }
}

fn run_build(input_path: &Path, output_path: Option<&Path>, minify: bool) -> ExitCode {
    let source = match fs::read_to_string(input_path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("Error reading '{}': {err}", input_path.display());
            return ExitCode::FAILURE;
        }
    };

    let filename = input_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("input.py")
        .to_string();

    let options = CompilerOptions {
        filename: filename.clone(),
        minify,
        ..Default::default()
    };

    match Compiler::compile(&source, &options) {
        Ok(artifact) => {
            let out_file = output_path.map(PathBuf::from).unwrap_or_else(|| {
                let stem = input_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("output");
                let ext = artifact.program_kind.default_extension();
                input_path.with_file_name(format!("{stem}.{ext}"))
            });

            if let Err(err) = fs::write(&out_file, &artifact.lua) {
                eprintln!("Error writing '{}': {err}", out_file.display());
                return ExitCode::FAILURE;
            }

            // Print success summary
            let target_desc = match artifact.program_kind {
                qsys_compiler::ProgramKind::Plugin => {
                    let name = artifact
                        .plugin_info
                        .as_ref()
                        .map(|i| i.name.as_str())
                        .unwrap_or("Plugin");
                    format!(
                        "Plugin '{name}' ({} controls, {} properties)",
                        artifact.control_count, artifact.property_count
                    )
                }
                qsys_compiler::ProgramKind::TextController => {
                    format!("Text Controller ({} controls)", artifact.control_count)
                }
                qsys_compiler::ProgramKind::ProceduralScript => "Procedural Script".to_string(),
            };

            println!("✓ Built {target_desc} -> {}", out_file.display());
            ExitCode::SUCCESS
        }
        Err(diags) => {
            let file =
                qsys_diagnostics::SourceFile::new(qsys_diagnostics::FileId(1), filename, source);
            print_diagnostics(&diags, &file);
            ExitCode::FAILURE
        }
    }
}

fn run_check(input_path: &Path) -> ExitCode {
    let source = match fs::read_to_string(input_path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("Error reading '{}': {err}", input_path.display());
            return ExitCode::FAILURE;
        }
    };

    let filename = input_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("input.py")
        .to_string();

    match Compiler::check(&source, &filename) {
        Ok(res) => {
            println!("✓ Checked '{}': No errors found.", res.file.name);
            ExitCode::SUCCESS
        }
        Err(diags) => {
            let file =
                qsys_diagnostics::SourceFile::new(qsys_diagnostics::FileId(1), filename, source);
            print_diagnostics(&diags, &file);
            ExitCode::FAILURE
        }
    }
}

fn run_gen_stubs(out_dir: &Path) -> ExitCode {
    match qsys_compiler::write_stubs_to_dir(out_dir) {
        Ok(_) => {
            println!("✓ Generated QSysPy type stubs in '{}':", out_dir.display());
            println!("  - {}/qsys.pyi", out_dir.display());
            println!("  - {}/qsys/__init__.pyi", out_dir.display());
            println!("  - {}/qsys/py.typed", out_dir.display());
            println!("\n💡 VS Code / Pylance setup:");
            println!("Add to your .vscode/settings.json:");
            println!(
                "  \"python.analysis.extraPaths\": [\"./{}\"]",
                out_dir.display()
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("Error writing stubs to '{}': {err}", out_dir.display());
            ExitCode::FAILURE
        }
    }
}

fn print_diagnostics(diags: &Diagnostics, file: &SourceFile) {
    for diag in &diags.items {
        let (line, col) = file.location(diag.span.start);
        eprintln!(
            "error[{}]: {} at {}:{}:{}",
            diag.code(),
            diag.message(),
            file.name,
            line,
            col
        );
        if let Some(help) = diag.help() {
            eprintln!("  = help: {help}");
        }
    }
}
