use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use qsys_diagnostics::Span;
use qsys_model::{ControlDef, ControlMember, PageLayout, PluginInfo, PropertyDef};

pub use crate::id::{ControlId, FunctionId, PropertyId, SymbolId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IRProgram {
    Plugin(Box<IRPlugin>),
    Script(Box<IRScript>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IRControlBinding {
    pub id: ControlId,
    pub ident: String,
    pub def: ControlDef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IRPropertyBinding {
    pub id: PropertyId,
    pub ident: String,
    pub def: PropertyDef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IRClass {
    pub name: String,
    pub base: Option<String>,
    pub class_vars: Vec<(String, IRExpr)>,
    pub emit_init_method: bool,
    pub constructor: Option<IRFunction>,
    pub methods: Vec<IRFunction>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IRPlugin {
    pub info: PluginInfo,
    pub properties: Vec<IRPropertyBinding>,
    pub controls: Vec<IRControlBinding>,
    pub pages: Vec<PageLayout>,
    pub class_name: String,
    pub classes: Vec<IRClass>,
    pub methods: Vec<IRFunction>,
    pub event_bindings: Vec<IREventBinding>,
    pub init_body: Vec<IRStmt>,
    pub symbol_names: FxHashMap<SymbolId, String>,
    pub top_level_stmts: Vec<IRStmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IRScript {
    pub name: String,
    pub is_controller: bool,
    pub bound_pins: Vec<String>,
    pub controls: Vec<IRControlBinding>,
    pub classes: Vec<IRClass>,
    pub methods: Vec<IRFunction>,
    pub event_bindings: Vec<IREventBinding>,
    pub symbol_names: FxHashMap<SymbolId, String>,
    pub top_level_stmts: Vec<IRStmt>,
    pub body: Vec<IRStmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IREventBinding {
    pub control_id: ControlId,
    pub control_name: String,
    pub channel_index: Option<usize>,
    pub member: ControlMember,
    pub handler_id: FunctionId,
    pub handler_name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IRParam {
    pub id: SymbolId,
    pub name: String,
    pub default: Option<IRExpr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IRFunction {
    pub id: FunctionId,
    pub name: String,
    pub params: Vec<IRParam>,
    pub body: Vec<IRStmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IRStmt {
    Assign {
        target: IRExpr,
        value: IRExpr,
        span: Span,
    },
    If {
        condition: IRExpr,
        truthiness: IRTruthiness,
        then_branch: Vec<IRStmt>,
        else_branch: Vec<IRStmt>,
        span: Span,
    },
    While {
        condition: IRExpr,
        truthiness: IRTruthiness,
        body: Vec<IRStmt>,
        span: Span,
    },
    For {
        target_id: SymbolId,
        target_name: String,
        iterable: IRExpr,
        body: Vec<IRStmt>,
        span: Span,
    },
    Return {
        value: Option<IRExpr>,
        span: Span,
    },
    Break {
        span: Span,
    },
    Expr {
        expr: IRExpr,
        span: Span,
    },
    RawLua {
        code: String,
        span: Span,
    },
    /// Emits solved Q-SYS `graphics` and `layout` table entries for a `PageLayout`.
    EmitPageLayout {
        page: PageLayout,
        span: Span,
    },

    // [`IRFunction`] and [`IRClass`] already have a `span` field.
    FunctionDef(IRFunction),
    ClassDef(IRClass),
    ModuleDef {
        name: String,
        body: Vec<IRStmt>,
        exports: Vec<String>,
        span: Span,
    },
    ImportBinding {
        local_name: String,
        target: IRExpr,
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IRTruthiness {
    Direct,
    NumericNotZero,
    StringNonEmpty,
    CollectionNonEmpty,
    AlwaysFalse,
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IRExpr {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Nil,

    // Authoritative Resolved References
    Local(SymbolId),
    Control(ControlId),
    Property(PropertyId),
    Function(FunctionId),
    Global(String),

    Tuple(Vec<IRExpr>),
    List(Vec<IRExpr>),
    Dict(Vec<(IRExpr, IRExpr)>),

    Binary {
        left: Box<IRExpr>,
        op: IRBinaryOp,
        right: Box<IRExpr>,
    },
    LogicalAnd {
        left: Box<IRExpr>,
        right: Box<IRExpr>,
        left_truthiness: IRTruthiness,
    },
    LogicalOr {
        left: Box<IRExpr>,
        right: Box<IRExpr>,
        left_truthiness: IRTruthiness,
    },
    Unary {
        op: IRUnaryOp,
        operand: Box<IRExpr>,
        truthiness: IRTruthiness,
    },

    Call {
        callee: Box<IRExpr>,
        args: Vec<IRExpr>,
        kwargs: Vec<(String, IRExpr)>,
    },
    /// Method call on an object instance (emitted as `object:method(...)` in Lua)
    MethodCall {
        object: Box<IRExpr>,
        method: String,
        args: Vec<IRExpr>,
        kwargs: Vec<(String, IRExpr)>,
    },
    Attribute {
        object: Box<IRExpr>,
        attr: String,
    },
    Index {
        base: Box<IRExpr>,
        index: Box<IRExpr>,
        is_zero_based: bool,
    },
    Lambda {
        params: Vec<IRParam>,
        body: Box<IRExpr>,
    },

    RawLua(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IRBinaryOp {
    Add,
    StringConcat,
    StringRep,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    Concat,
    BitAnd,
    BitOr,
    BitXor,
    LShift,
    RShift,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IRUnaryOp {
    Not,
    Neg,
    Pos,
    BitNot,
    Len,
}
