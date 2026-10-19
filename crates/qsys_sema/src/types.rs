use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use qsys_diagnostics::Span;
use qsys_ir::{ControlId, FunctionId, PropertyId, SymbolId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolvedTarget {
    Local(SymbolId),
    Control(ControlId),
    Property(PropertyId),
    Function(FunctionId),
    Global(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Type {
    Int,
    Float,
    Bool,
    String,
    None,

    List(Box<Type>),
    Dict(Box<Type>, Box<Type>),

    ControlHandle,
    PropertyHandle,

    // Q-SYS Native Runtime Objects
    UdpSocketClass,
    UdpSocketHandle,
    TcpSocketClass,
    TcpSocketHandle,
    TimerClass,
    TimerHandle,

    ClassInstance { name: String }, // QSysPlugin instance (`self`)
    UserClass { name: String },     // Custom class constructor (e.g. `Device`)
    UserInstance { name: String },  // Custom class instance object (e.g. `dev = Device(...)`)

    Any,
}

impl Type {
    pub fn is_numeric(&self) -> bool {
        matches!(self, Self::Int | Self::Float)
    }

    pub fn truthiness_strategy(&self) -> TruthinessStrategy {
        match self {
            Self::Bool
            | Self::ControlHandle
            | Self::UdpSocketHandle
            | Self::TcpSocketHandle
            | Self::TimerHandle
            | Self::ClassInstance { .. }
            | Self::UserInstance { .. } => TruthinessStrategy::Direct,

            Self::Int | Self::Float => TruthinessStrategy::NumericNotZero,
            Self::String => TruthinessStrategy::StringNonEmpty,
            Self::List(_) | Self::Dict(_, _) => TruthinessStrategy::CollectionNonEmpty,
            Self::None => TruthinessStrategy::AlwaysFalse,
            Self::Any
            | Self::PropertyHandle
            | Self::UdpSocketClass
            | Self::TcpSocketClass
            | Self::TimerClass
            | Self::UserClass { .. } => TruthinessStrategy::Dynamic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TruthinessStrategy {
    Direct,
    NumericNotZero,
    StringNonEmpty,
    CollectionNonEmpty,
    AlwaysFalse,
    Dynamic,
}

#[derive(Debug, Clone, Default)]
pub struct TypeMap {
    types: FxHashMap<Span, Type>,
    truthiness: FxHashMap<Span, TruthinessStrategy>,
    resolutions: FxHashMap<Span, ResolvedTarget>,
    symbol_names: FxHashMap<SymbolId, String>,
    class_bases: FxHashMap<String, String>,
    class_member_types: FxHashMap<(String, String), Type>,
    function_return_types: FxHashMap<String, Type>,
}

impl TypeMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_symbol_name(&mut self, id: SymbolId, name: impl Into<String>) {
        self.symbol_names.insert(id, name.into());
    }

    pub fn symbol_names(&self) -> &FxHashMap<SymbolId, String> {
        &self.symbol_names
    }

    pub fn insert_class_base(
        &mut self,
        class_name: impl Into<String>,
        base_name: impl Into<String>,
    ) {
        self.class_bases.insert(class_name.into(), base_name.into());
    }

    pub fn get_class_base(&self, class_name: &str) -> Option<&str> {
        self.class_bases.get(class_name).map(String::as_str)
    }

    pub fn insert_class_member_type(
        &mut self,
        class_name: impl Into<String>,
        member_name: impl Into<String>,
        ty: Type,
    ) {
        self.class_member_types
            .insert((class_name.into(), member_name.into()), ty);
    }

    pub fn get_class_member_type(&self, class_name: &str, member_name: &str) -> Option<&Type> {
        let mut current = class_name;
        for _ in 0..16 {
            if let Some(ty) = self
                .class_member_types
                .get(&(current.to_string(), member_name.to_string()))
            {
                return Some(ty);
            }
            if let Some(base) = self.class_bases.get(current) {
                current = base;
            } else {
                break;
            }
        }
        None
    }

    pub fn class_member_names(&self, class_name: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut current = class_name;
        for _ in 0..16 {
            for (cls, member) in self.class_member_types.keys() {
                if cls == current && member != "__init__" && !names.contains(member) {
                    names.push(member.clone());
                }
            }
            if let Some(base) = self.class_bases.get(current) {
                current = base;
            } else {
                break;
            }
        }
        names.sort();
        names
    }

    pub fn insert_function_return_type(&mut self, fn_name: impl Into<String>, ty: Type) {
        self.function_return_types.insert(fn_name.into(), ty);
    }

    pub fn get_function_return_type(&self, fn_name: &str) -> Option<&Type> {
        self.function_return_types.get(fn_name)
    }

    pub fn insert_type(&mut self, span: Span, ty: Type) {
        self.truthiness.insert(span, ty.truthiness_strategy());
        self.types.insert(span, ty);
    }

    pub fn insert_resolution(&mut self, span: Span, target: ResolvedTarget) {
        self.resolutions.insert(span, target);
    }

    pub fn get_type(&self, span: Span) -> Option<&Type> {
        self.types.get(&span)
    }

    /// Access all inferred types across the module
    pub fn types(&self) -> &FxHashMap<Span, Type> {
        &self.types
    }

    /// Access all resolved targets across the module
    pub fn resolutions(&self) -> &FxHashMap<Span, ResolvedTarget> {
        &self.resolutions
    }

    pub fn get_resolution(&self, span: Span) -> Option<&ResolvedTarget> {
        self.resolutions.get(&span)
    }

    pub fn get_truthiness(&self, span: Span) -> TruthinessStrategy {
        self.truthiness
            .get(&span)
            .copied()
            .unwrap_or(TruthinessStrategy::Dynamic)
    }
}
