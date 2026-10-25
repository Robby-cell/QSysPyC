use rustc_hash::{FxHashMap, FxHashSet};

use qsys_diagnostics::Span;
use qsys_ir::{ControlId, FunctionId, PropertyId, SymbolId};
use qsys_model::{ControlDef, PropertyDef};

use crate::types::Type;

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum SymbolKind {
    Variable {
        id: SymbolId,
        ty: Type,
    },
    Parameter {
        id: SymbolId,
        index: usize,
        ty: Type,
    },
    Control {
        id: ControlId,
        def: ControlDef,
    },
    Property {
        id: PropertyId,
        def: PropertyDef,
    },
    Method {
        id: FunctionId,
        param_count: usize,
        has_self: bool,
    },
    Class {
        name: String,
        base: Option<String>,
    },
    UserClass {
        name: String,
    },
    Module {
        name: String,
    },
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    pub span: Span,
}

impl Symbol {
    pub fn new(id: SymbolId, name: impl Into<String>, kind: SymbolKind, span: Span) -> Self {
        Self {
            id,
            name: name.into(),
            kind,
            span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeKind {
    Module,
    Class { name: String },
    Function { name: String },
    Block,
}

#[derive(Debug, Clone)]
pub struct Scope {
    pub kind: ScopeKind,
    pub symbols: FxHashMap<String, Symbol>,
    pub explicit_globals: FxHashSet<String>,
}

impl Scope {
    pub fn new(kind: ScopeKind) -> Self {
        Self {
            kind,
            symbols: FxHashMap::default(),
            explicit_globals: FxHashSet::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScopeStack {
    scopes: Vec<Scope>,
    next_symbol: u32,
    next_control: u32,
    next_property: u32,
    next_function: u32,
}

impl Default for ScopeStack {
    fn default() -> Self {
        Self {
            scopes: vec![Scope::new(ScopeKind::Module)],
            next_symbol: 0,
            next_control: 0,
            next_property: 0,
            next_function: 0,
        }
    }
}

impl ScopeStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alloc_symbol_id(&mut self) -> SymbolId {
        let id = SymbolId(self.next_symbol);
        self.next_symbol += 1;
        id
    }

    pub fn alloc_control_id(&mut self) -> ControlId {
        let id = ControlId(self.next_control);
        self.next_control += 1;
        id
    }

    pub fn alloc_property_id(&mut self) -> PropertyId {
        let id = PropertyId(self.next_property);
        self.next_property += 1;
        id
    }

    pub fn alloc_function_id(&mut self) -> FunctionId {
        let id = FunctionId(self.next_function);
        self.next_function += 1;
        id
    }

    pub fn push_scope(&mut self, kind: ScopeKind) {
        self.scopes.push(Scope::new(kind));
    }

    pub fn pop_scope(&mut self) -> Option<Scope> {
        if self.scopes.len() > 1 {
            self.scopes.pop()
        } else {
            None
        }
    }

    pub fn current_scope(&self) -> &Scope {
        self.scopes
            .last()
            .expect("ScopeStack must have at least one scope")
    }

    pub fn current_scope_mut(&mut self) -> &mut Scope {
        self.scopes
            .last_mut()
            .expect("ScopeStack must have at least one scope")
    }

    pub fn define(&mut self, symbol: Symbol) {
        self.current_scope_mut()
            .symbols
            .insert(symbol.name.clone(), symbol);
    }

    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        if self.is_explicit_global(name)
            && let Some(sym) = self.lookup_in_module(name)
        {
            return Some(sym);
        }
        for scope in self.scopes.iter().rev() {
            if let Some(symbol) = scope.symbols.get(name) {
                return Some(symbol);
            }
        }
        None
    }

    pub fn is_explicit_global(&self, name: &str) -> bool {
        for scope in self.scopes.iter().rev() {
            if scope.explicit_globals.contains(name) {
                return true;
            }
            if matches!(
                scope.kind,
                ScopeKind::Function { .. } | ScopeKind::Class { .. } | ScopeKind::Module
            ) {
                break;
            }
        }
        false
    }

    pub fn lookup_in_module(&self, name: &str) -> Option<&Symbol> {
        let mod_idx = self
            .scopes
            .iter()
            .rposition(|s| matches!(s.kind, ScopeKind::Module))
            .unwrap_or(0);
        self.scopes[mod_idx].symbols.get(name)
    }

    pub fn define_or_lookup_global(&mut self, name: &str, span: Span) -> (SymbolId, Type) {
        let in_fn_or_block = matches!(
            self.current_scope().kind,
            ScopeKind::Function { .. } | ScopeKind::Block
        );
        if in_fn_or_block {
            self.current_scope_mut()
                .explicit_globals
                .insert(name.to_string());
        }

        let mod_idx = self
            .scopes
            .iter()
            .rposition(|s| matches!(s.kind, ScopeKind::Module))
            .unwrap_or(0);

        let (sym_id, ty) = if let Some(existing) = self.scopes[mod_idx].symbols.get(name) {
            match &existing.kind {
                SymbolKind::Variable { id, ty } | SymbolKind::Parameter { id, ty, .. } => {
                    (*id, ty.clone())
                }
                _ => (existing.id, Type::Any),
            }
        } else {
            let sym_id = self.alloc_symbol_id();
            self.scopes[mod_idx].symbols.insert(
                name.to_string(),
                Symbol::new(
                    sym_id,
                    name.to_string(),
                    SymbolKind::Variable {
                        id: sym_id,
                        ty: Type::Any,
                    },
                    span,
                ),
            );
            (sym_id, Type::Any)
        };

        if in_fn_or_block && self.scopes.len() - 1 != mod_idx {
            self.current_scope_mut().symbols.insert(
                name.to_string(),
                Symbol::new(
                    sym_id,
                    name.to_string(),
                    SymbolKind::Variable {
                        id: sym_id,
                        ty: ty.clone(),
                    },
                    span,
                ),
            );
        }

        (sym_id, ty)
    }

    pub fn lookup_nonlocal(&self, name: &str) -> Option<&Symbol> {
        for scope in self.scopes.iter().rev().skip(1) {
            if let Some(symbol) = scope.symbols.get(name) {
                return Some(symbol);
            }
        }
        None
    }

    /// Updates the variable type across all scopes containing `target_id`.
    pub fn update_variable_type(&mut self, target_id: SymbolId, new_ty: Type) {
        for scope in self.scopes.iter_mut().rev() {
            for symbol in scope.symbols.values_mut() {
                if symbol.id == target_id
                    && let SymbolKind::Variable { ty, .. } = &mut symbol.kind
                {
                    *ty = new_ty.clone();
                }
            }
        }
    }

    pub fn all_visible_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for scope in self.scopes.iter().rev() {
            for key in scope.symbols.keys() {
                if !names.contains(key) {
                    names.push(key.clone());
                }
            }
        }
        names
    }

    /// Collects all control and property names declared in the current class scope.
    pub fn class_member_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for scope in self.scopes.iter().rev() {
            if matches!(scope.kind, ScopeKind::Class { .. }) {
                for (name, sym) in &scope.symbols {
                    if matches!(
                        sym.kind,
                        SymbolKind::Control { .. }
                            | SymbolKind::Property { .. }
                            | SymbolKind::Method { .. }
                    ) {
                        names.push(name.clone());
                    }
                }
                break;
            }
        }
        names
    }
}
