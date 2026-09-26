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

    ClassInstance { name: String },

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
            | Self::ClassInstance { .. } => TruthinessStrategy::Direct,

            Self::Int | Self::Float => TruthinessStrategy::NumericNotZero,
            Self::String => TruthinessStrategy::StringNonEmpty,
            Self::List(_) | Self::Dict(_, _) => TruthinessStrategy::CollectionNonEmpty,
            Self::None => TruthinessStrategy::AlwaysFalse,
            Self::Any
            | Self::PropertyHandle
            | Self::UdpSocketClass
            | Self::TcpSocketClass
            | Self::TimerClass => TruthinessStrategy::Dynamic,
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
}

impl TypeMap {
    pub fn new() -> Self {
        Self::default()
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
