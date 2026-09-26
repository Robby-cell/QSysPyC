use std::sync::Arc;

use qsys_codegen_lua::LuaBackend;
use qsys_diagnostics::{DiagnosticKind, Diagnostics, FileId, SourceFile, Span};
use qsys_ir::{Backend, CodegenConfig};
use qsys_parser::parse_module;
use qsys_sema::{TargetKind, analyze};

use crate::options::{CheckResult, CompiledArtifact, CompilerOptions, ProgramKind};

pub struct Compiler;

impl Compiler {
    /// Compiles raw Python source text into a ready-to-deploy Q-SYS Lua artifact.
    pub fn compile(
        source: &str,
        options: &CompilerOptions,
    ) -> Result<CompiledArtifact, Diagnostics> {
        let file = SourceFile::new(FileId(1), &options.filename, source);
        Self::compile_file(file, options)
    }

    /// Compiles an in-memory `SourceFile` into a Q-SYS Lua artifact.
    pub fn compile_file(
        file: Arc<SourceFile>,
        options: &CompilerOptions,
    ) -> Result<CompiledArtifact, Diagnostics> {
        let mut all_diagnostics = Diagnostics::new();

        // Parsing (Python text -> AST)
        let parsed = parse_module(file.clone())?;

        // Semantic Analysis (AST -> Semantic Model with stable IDs)
        let model = analyze(&parsed)?;

        // Extract metadata for reports & CLI summaries
        let (program_kind, plugin_info, control_count, property_count) = match &model.target {
            TargetKind::Plugin(p) => (
                ProgramKind::Plugin,
                Some(p.info.clone()),
                p.controls.len(),
                p.properties.len(),
            ),
            TargetKind::TextController(c) => {
                (ProgramKind::TextController, None, c.controls.len(), 0)
            }
            TargetKind::ProceduralScript(_) => (ProgramKind::ProceduralScript, None, 0, 0),
        };

        // Lowering (Semantic Model -> Typed IR)
        let ir = model.into_ir();

        // Code Generation (IR -> Q-SYS Lua 5.3)
        let backend = LuaBackend::new();
        let codegen_config = CodegenConfig {
            emit_source_maps: options.emit_source_maps,
            minify: options.minify,
            target_name: options.target_name.clone().unwrap_or_else(|| {
                plugin_info
                    .as_ref()
                    .map(|i| i.name.clone())
                    .unwrap_or_else(|| "main".to_string())
            }),
        };

        let (lua, codegen_diags) = match backend.emit(&ir, &codegen_config) {
            Ok(res) => res,
            Err(err) => {
                all_diagnostics.report(
                    DiagnosticKind::syntax_error(format!("Codegen error: {err}")),
                    Span::empty(),
                );
                return Err(all_diagnostics);
            }
        };

        all_diagnostics.items.extend(codegen_diags.items);

        Ok(CompiledArtifact {
            file,
            lua,
            program_kind,
            diagnostics: all_diagnostics,
            plugin_info,
            control_count,
            property_count,
        })
    }

    /// Checks source code syntax and semantics without performing code generation.
    pub fn check(source: &str, filename: impl Into<String>) -> Result<CheckResult, Diagnostics> {
        let file = SourceFile::new(FileId(1), filename.into(), source);
        Self::check_file(file)
    }

    /// Checks an in-memory `SourceFile` without performing code generation.
    pub fn check_file(file: Arc<SourceFile>) -> Result<CheckResult, Diagnostics> {
        let parsed = parse_module(file.clone())?;
        let model = analyze(&parsed)?;

        let (program_kind, plugin_info, control_count, property_count) = match &model.target {
            TargetKind::Plugin(p) => (
                ProgramKind::Plugin,
                Some(p.info.clone()),
                p.controls.len(),
                p.properties.len(),
            ),
            TargetKind::TextController(c) => {
                (ProgramKind::TextController, None, c.controls.len(), 0)
            }
            TargetKind::ProceduralScript(_) => (ProgramKind::ProceduralScript, None, 0, 0),
        };

        Ok(CheckResult {
            file,
            program_kind,
            diagnostics: Diagnostics::new(),
            plugin_info,
            control_count,
            property_count,
        })
    }
}
