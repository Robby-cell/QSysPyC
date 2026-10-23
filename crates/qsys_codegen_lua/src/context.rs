use rustc_hash::{FxHashMap, FxHashSet};

use qsys_ir::{ControlId, FunctionId, PropertyId, SymbolId};
use qsys_model::{ControlDef, PropertyDef};

#[derive(Debug, Clone, Default)]
pub struct CodegenContext {
    pub controls: FxHashMap<ControlId, ControlDef>,
    pub properties: FxHashMap<PropertyId, PropertyDef>,
    pub functions: FxHashMap<FunctionId, String>,
    pub symbol_names: FxHashMap<SymbolId, String>,
    pub declared_locals: FxHashSet<SymbolId>,
    pub method_handler_controls: FxHashMap<String, ControlId>,
    pub active_handler_control: Option<ControlId>,
    pub local_controls: FxHashMap<SymbolId, ControlId>,
    pub needs_truthy_helper: bool,
    pub needs_generic_data_helper: bool,
}

impl CodegenContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sanitize_ident(name: &str) -> String {
        match name {
            "and" | "break" | "do" | "else" | "elseif" | "end" | "false" | "for" | "function"
            | "goto" | "if" | "in" | "local" | "nil" | "not" | "or" | "repeat" | "return"
            | "then" | "true" | "until" | "while" => {
                format!("_{name}")
            }
            _ => name.to_string(),
        }
    }

    pub fn get_symbol_name(&self, id: SymbolId) -> String {
        if let Some(name) = self.symbol_names.get(&id) {
            Self::sanitize_ident(name)
        } else {
            format!("_v{}", id.0)
        }
    }
}
