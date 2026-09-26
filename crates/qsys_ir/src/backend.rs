use qsys_diagnostics::Diagnostics;

use crate::ir::IRProgram;

/// Configuration options passed to backends during code generation.
#[derive(Debug, Clone)]
pub struct CodegenConfig {
    pub emit_source_maps: bool,
    pub minify: bool,
    pub target_name: String,
}

impl Default for CodegenConfig {
    fn default() -> Self {
        Self {
            emit_source_maps: false,
            minify: false,
            target_name: "plugin".to_string(),
        }
    }
}

/// The abstraction implemented by all code generation targets.
pub trait Backend {
    /// The resulting artifact type (e.g. `String` for Lua source code).
    type Output;
    /// Backend-specific error.
    type Error: std::error::Error + Send + Sync + 'static;

    /// The unique identifier of this backend (e.g. "lua53", "docgen").
    fn name(&self) -> &'static str;

    /// Lower the language-independent IR into the target representation.
    fn emit(
        &self,
        program: &IRProgram,
        config: &CodegenConfig,
    ) -> Result<(Self::Output, Diagnostics), Self::Error>;
}
