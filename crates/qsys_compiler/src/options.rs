use std::sync::Arc;

use qsys_diagnostics::{Diagnostics, EditorMarker, SourceFile};
use qsys_model::PluginInfo;
use qsys_runtime::RuntimeEnv;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramKind {
    Plugin,
    TextController,
    ProceduralScript,
}

impl ProgramKind {
    pub fn default_extension(&self) -> &'static str {
        match self {
            Self::Plugin => "qplug",
            Self::TextController | Self::ProceduralScript => "lua",
        }
    }
}

/// User-configurable options for the compiler pipeline.
#[derive(Debug, Clone)]
pub struct CompilerOptions {
    pub target_name: Option<String>,
    pub emit_source_maps: bool,
    pub minify: bool,
    pub filename: String,
    /// Runtime environment (globals, native types, removed builtins) to compile against.
    pub env: Arc<RuntimeEnv>,
}

impl Default for CompilerOptions {
    fn default() -> Self {
        Self {
            target_name: None,
            emit_source_maps: false,
            minify: false,
            filename: "main.py".to_string(),
            env: RuntimeEnv::qsys(),
        }
    }
}

/// The result of a successful compilation.
#[derive(Debug, Clone)]
pub struct CompiledArtifact {
    pub file: Arc<SourceFile>,
    pub lua: String,
    pub program_kind: ProgramKind,
    pub diagnostics: Diagnostics,
    pub plugin_info: Option<PluginInfo>,
    pub control_count: usize,
    pub property_count: usize,
}

impl CompiledArtifact {
    pub fn to_editor_markers(&self) -> Vec<EditorMarker> {
        self.diagnostics.to_editor_markers(&self.file)
    }

    pub fn to_editor_markers_json(&self) -> String {
        serde_json::to_string(&self.to_editor_markers()).unwrap_or_else(|_| "[]".to_string())
    }
}

/// The result of checking code validity without running codegen.
#[derive(Debug, Clone)]
pub struct CheckResult {
    pub file: Arc<SourceFile>,
    pub program_kind: ProgramKind,
    pub diagnostics: Diagnostics,
    pub plugin_info: Option<PluginInfo>,
    pub control_count: usize,
    pub property_count: usize,
}

impl CheckResult {
    pub fn to_editor_markers(&self) -> Vec<EditorMarker> {
        self.diagnostics.to_editor_markers(&self.file)
    }

    pub fn to_editor_markers_json(&self) -> String {
        serde_json::to_string(&self.to_editor_markers()).unwrap_or_else(|_| "[]".to_string())
    }
}
