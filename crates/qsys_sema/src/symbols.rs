use rustc_hash::FxHashMap;

use qsys_diagnostics::Span;
use qsys_ir::{ControlId, FunctionId, PropertyId, SymbolId};
use qsys_model::{ControlDef, PropertyDef};

use crate::types::Type;

#[derive(Debug, Clone)]
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
}

impl Scope {
    pub fn new(kind: ScopeKind) -> Self {
        Self {
            kind,
            symbols: FxHashMap::default(),
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
        for scope in self.scopes.iter().rev() {
            if let Some(symbol) = scope.symbols.get(name) {
                return Some(symbol);
            }
        }
        None
    }

    /// Collects all symbol names visible from the current innermost scope up to the root.
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
