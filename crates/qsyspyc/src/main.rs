mod cli;
mod lsp;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

use qsys_compiler::{Compiler, CompilerOptions};
use qsys_diagnostics::{Diagnostics, SourceFile};

use crate::cli::{Cli, Commands};

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
    let raw_source = match fs::read_to_string(input_path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("Error reading '{}': {err}", input_path.display());
            return ExitCode::FAILURE;
        }
    };
    let source = bundle_local_imports(input_path, &raw_source);

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
    let raw_source = match fs::read_to_string(input_path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("Error reading '{}': {err}", input_path.display());
            return ExitCode::FAILURE;
        }
    };
    let source = bundle_local_imports(input_path, &raw_source);

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

fn extract_import_specs(src: &str) -> Vec<String> {
    let mut specs = Vec::new();
    for line in src.lines() {
        let trimmed = line.trim_start();
        if let Some(after_from) = trimmed.strip_prefix("from ") {
            if let Some((spec, _)) = after_from.split_once(" import ") {
                let s = spec.trim();
                if !s.is_empty() && s != "qsys" && !s.starts_with("qsys.") {
                    specs.push(s.to_string());
                }
            }
        } else if let Some(after_imp) = trimmed.strip_prefix("import ") {
            for part in after_imp.split(',') {
                let token = part.split_whitespace().next().unwrap_or("");
                if !token.is_empty() && token != "qsys" && !token.starts_with("qsys.") {
                    specs.push(token.to_string());
                }
            }
        }
    }
    specs
}

fn resolve_local_module_path(from_file: &Path, spec: &str) -> Option<PathBuf> {
    let base_dir = from_file.parent().unwrap_or_else(|| Path::new("."));
    let clean_spec = spec.trim_start_matches('.').replace('.', "/");
    if clean_spec.is_empty() {
        return None;
    }
    let candidates = [
        base_dir.join(format!("{clean_spec}.py")),
        base_dir.join(&clean_spec).join("__init__.py"),
    ];
    candidates
        .into_iter()
        .find(|cand| cand.is_file() && cand != from_file)
}

fn bundle_local_imports(entry_path: &Path, entry_source: &str) -> String {
    let mut visited = HashSet::new();
    if let Ok(canon) = entry_path.canonicalize() {
        visited.insert(canon);
    }
    let mut ordered_modules: Vec<(PathBuf, String)> = Vec::new();

    fn visit(
        file_path: &Path,
        src: &str,
        visited: &mut HashSet<PathBuf>,
        ordered: &mut Vec<(PathBuf, String)>,
    ) {
        for spec in extract_import_specs(src) {
            if let Some(resolved) = resolve_local_module_path(file_path, &spec) {
                let key = resolved.canonicalize().unwrap_or_else(|_| resolved.clone());
                if visited.insert(key)
                    && let Ok(mod_src) = fs::read_to_string(&resolved)
                {
                    visit(&resolved, &mod_src, visited, ordered);
                    ordered.push((resolved, mod_src));
                }
            }
        }
    }

    visit(entry_path, entry_source, &mut visited, &mut ordered_modules);

    if ordered_modules.is_empty() {
        return entry_source.to_string();
    }

    let mut out = String::new();
    for (mod_path, mod_src) in ordered_modules {
        let display_name = mod_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("module.py");
        out.push_str(&format!("# --- Bundled module: {display_name} ---\n"));
        out.push_str(&mod_src);
        if !mod_src.ends_with('\n') {
            out.push('\n');
        }
    }
    out.push_str("# --- End bundled modules ---\n");
    out.push_str(entry_source);
    out
}
