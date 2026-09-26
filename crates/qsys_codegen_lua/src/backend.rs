use thiserror::Error;

use qsys_diagnostics::Diagnostics;
use qsys_ir::{Backend, CodegenConfig, ControlId, IRProgram, PropertyId};

use crate::context::CodegenContext;
use crate::plugin::emit_plugin;
use crate::script::emit_script;

#[derive(Debug, Error)]
pub enum LuaCodegenError {
    #[error("Unresolved control with ID {0:?}")]
    UnresolvedControl(ControlId),
    #[error("Unresolved property with ID {0:?}")]
    UnresolvedProperty(PropertyId),
    #[error("Code generation error: {0}")]
    Custom(String),
}

#[derive(Debug, Default, Clone)]
pub struct LuaBackend;

impl LuaBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Backend for LuaBackend {
    type Output = String;
    type Error = LuaCodegenError;

    fn name(&self) -> &'static str {
        "lua53"
    }

    fn emit(
        &self,
        program: &IRProgram,
        _config: &CodegenConfig,
    ) -> Result<(Self::Output, Diagnostics), Self::Error> {
        let mut ctx = CodegenContext::new();
        let diagnostics = Diagnostics::new();

        let lua_code = match program {
            IRProgram::Plugin(plugin) => emit_plugin(plugin, &mut ctx),
            IRProgram::Script(script) => emit_script(script, &mut ctx),
        };

        Ok((lua_code, diagnostics))
    }
}
