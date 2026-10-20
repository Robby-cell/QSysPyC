//! Declarative building blocks used by [`RuntimePlugin`](crate::RuntimePlugin)s to describe
//! the globals, native object types and builtin lowerings available on a target runtime.

use serde::{Deserialize, Serialize};

/// A target-agnostic reference to a type, as declared by a runtime plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeRef {
    Any,
    None,
    Bool,
    Int,
    Float,
    Str,
    List(Box<TypeRef>),
    Dict(Box<TypeRef>, Box<TypeRef>),
    /// A native object type registered with [`EnvBuilder::add_type`](crate::EnvBuilder::add_type),
    /// or one of the DSL-owned handle types (`ControlHandle`, `PropertyHandle`).
    Object(String),
    /// A callback signature (used for event fields such as `TimerHandle.EventHandler`).
    Callable(Vec<TypeRef>),
}

impl TypeRef {
    pub fn object(name: impl Into<String>) -> Self {
        Self::Object(name.into())
    }

    pub fn list(elem: TypeRef) -> Self {
        Self::List(Box::new(elem))
    }

    pub fn dict(key: TypeRef, value: TypeRef) -> Self {
        Self::Dict(Box::new(key), Box::new(value))
    }

    pub fn callable(args: impl IntoIterator<Item = TypeRef>) -> Self {
        Self::Callable(args.into_iter().collect())
    }

    /// Renders the type as a Python annotation (used by stubs, hover and signature help).
    pub fn python_annotation(&self) -> String {
        match self {
            Self::Any => "Any".to_string(),
            Self::None => "None".to_string(),
            Self::Bool => "bool".to_string(),
            Self::Int => "int".to_string(),
            Self::Float => "float".to_string(),
            Self::Str => "str".to_string(),
            Self::List(elem) => format!("list[{}]", elem.python_annotation()),
            Self::Dict(k, v) => {
                format!("dict[{}, {}]", k.python_annotation(), v.python_annotation())
            }
            Self::Object(name) => name.clone(),
            Self::Callable(args) => format!(
                "Callable[[{}], None]",
                args.iter()
                    .map(TypeRef::python_annotation)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// Where a global may legally be referenced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Availability {
    #[default]
    Always,
    Runtime,
    DesignTime,
}

impl Availability {
    pub fn permits(self, context: ExecutionContext) -> bool {
        matches!(
            (self, context),
            (Self::Always, _)
                | (_, ExecutionContext::Unknown)
                | (Self::Runtime, ExecutionContext::Runtime)
                | (Self::DesignTime, ExecutionContext::DesignTime)
        )
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Always => "everywhere",
            Self::Runtime => "at runtime",
            Self::DesignTime => "at design time",
        }
    }
}

/// The phase in which a piece of analyzed code executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ExecutionContext {
    Runtime,
    DesignTime,
    /// Code that executes in both phases (e.g. module-level statements of a plugin).
    Both,
    /// Code whose execution phase cannot be determined statically (e.g. helper function bodies).
    #[default]
    Unknown,
}

impl ExecutionContext {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Runtime => "in runtime code",
            Self::DesignTime => "in design-time code",
            Self::Both => "in module-level code that runs at both design time and runtime",
            Self::Unknown => "here",
        }
    }
}

/// How a method is invoked in the emitted Lua.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CallStyle {
    /// `obj:method(...)` - instance methods on native handles.
    Colon,
    /// `obj.method(...)` - library functions and static constructors.
    #[default]
    Dot,
}

/// How a builtin function call is rewritten when lowering Python to Lua.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Lowering {
    /// Emitted unchanged: `print(x)` -> `print(x)`.
    #[default]
    Direct,
    /// Calls a differently named global: `str(x)` -> `tostring(x)`.
    Rename(String),
    /// Calls a function inside a library table: `abs(x)` -> `math.abs(x)`.
    Qualified { module: String, name: String },
    /// Lowers a single-argument call to Lua's length operator: `len(x)` -> `#x`.
    LengthOperator,
}

/// How a function's return type is computed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Returns {
    Type(TypeRef),
    /// `int` when every argument is an `int`, otherwise `float` (e.g. `abs`, `min`, `max`).
    NumericPromotion,
}

impl Default for Returns {
    fn default() -> Self {
        Self::Type(TypeRef::None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Param {
    pub name: String,
    pub ty: TypeRef,
    pub optional: bool,
    pub doc: String,
}

impl Param {
    pub fn new(name: impl Into<String>, ty: TypeRef) -> Self {
        Self {
            name: name.into(),
            ty,
            optional: false,
            doc: String::new(),
        }
    }

    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    pub fn doc(mut self, doc: impl Into<String>) -> Self {
        self.doc = doc.into();
        self
    }

    /// `name: type` or `name?: type` for optional parameters.
    pub fn label(&self) -> String {
        let marker = if self.optional { "?" } else { "" };
        format!("{}{marker}: {}", self.name, self.ty.python_annotation())
    }
}

/// A callable: either a global function or a method on an [`ObjectType`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    pub doc: String,
    pub params: Vec<Param>,
    /// Accepts any number of trailing positional arguments (`print(*values)`).
    pub variadic: bool,
    pub returns: Returns,
    pub call_style: CallStyle,
    pub lowering: Lowering,
    /// Optional IDE snippet inserted on completion (Monaco/LSP snippet syntax).
    pub snippet: Option<String>,
}

impl Function {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            doc: String::new(),
            params: Vec::new(),
            variadic: false,
            returns: Returns::default(),
            call_style: CallStyle::default(),
            lowering: Lowering::default(),
            snippet: None,
        }
    }

    pub fn doc(mut self, doc: impl Into<String>) -> Self {
        self.doc = doc.into();
        self
    }

    pub fn param(mut self, name: impl Into<String>, ty: TypeRef) -> Self {
        self.params.push(Param::new(name, ty));
        self
    }

    pub fn optional_param(mut self, name: impl Into<String>, ty: TypeRef) -> Self {
        self.params.push(Param::new(name, ty).optional());
        self
    }

    pub fn arg(mut self, param: Param) -> Self {
        self.params.push(param);
        self
    }

    pub fn variadic(mut self) -> Self {
        self.variadic = true;
        self
    }

    pub fn returns(mut self, ty: TypeRef) -> Self {
        self.returns = Returns::Type(ty);
        self
    }

    pub fn returns_numeric_promotion(mut self) -> Self {
        self.returns = Returns::NumericPromotion;
        self
    }

    pub fn colon(mut self) -> Self {
        self.call_style = CallStyle::Colon;
        self
    }

    pub fn dot(mut self) -> Self {
        self.call_style = CallStyle::Dot;
        self
    }

    pub fn lower_to(mut self, lowering: Lowering) -> Self {
        self.lowering = lowering;
        self
    }

    pub fn snippet(mut self, snippet: impl Into<String>) -> Self {
        self.snippet = Some(snippet.into());
        self
    }

    /// The return type rendered as a Python annotation.
    pub fn return_annotation(&self) -> String {
        match &self.returns {
            Returns::Type(ty) => ty.python_annotation(),
            Returns::NumericPromotion => "int | float".to_string(),
        }
    }

    /// Comma separated parameter labels (`ip: str, port: int`).
    pub fn params_label(&self) -> String {
        let mut parts: Vec<String> = self.params.iter().map(Param::label).collect();
        if self.variadic {
            parts.push("*args: Any".to_string());
        }
        parts.join(", ")
    }
}

/// A data member on an [`ObjectType`]. Callable-typed fields are rendered as events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub ty: TypeRef,
    pub doc: String,
}

impl Field {
    pub fn new(name: impl Into<String>, ty: TypeRef) -> Self {
        Self {
            name: name.into(),
            ty,
            doc: String::new(),
        }
    }

    pub fn doc(mut self, doc: impl Into<String>) -> Self {
        self.doc = doc.into();
        self
    }

    pub fn is_event(&self) -> bool {
        matches!(self.ty, TypeRef::Callable(_))
    }
}

/// A native runtime object (library table, class factory or handle).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectType {
    pub name: String,
    pub doc: String,
    pub fields: Vec<Field>,
    pub methods: Vec<Function>,
    /// Call style used for members that are not explicitly declared.
    pub default_call_style: CallStyle,
    /// Name of the plugin that registered this type.
    pub origin: String,
}

impl ObjectType {
    /// A library table or class factory whose members are invoked with `.` (e.g. `Timer`, `math`).
    pub fn library(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            doc: String::new(),
            fields: Vec::new(),
            methods: Vec::new(),
            default_call_style: CallStyle::Dot,
            origin: String::new(),
        }
    }

    /// A runtime handle whose methods are invoked with `:` (e.g. `TimerHandle`).
    pub fn handle(name: impl Into<String>) -> Self {
        Self {
            default_call_style: CallStyle::Colon,
            ..Self::library(name)
        }
    }

    pub fn doc(mut self, doc: impl Into<String>) -> Self {
        self.doc = doc.into();
        self
    }

    pub fn field(mut self, field: Field) -> Self {
        self.fields.push(field);
        self
    }

    /// Adds a method. Methods on handle types default to colon calls unless built with `.dot()`.
    pub fn method(mut self, mut method: Function) -> Self {
        if self.default_call_style == CallStyle::Colon && method.call_style == CallStyle::Dot {
            method.call_style = CallStyle::Colon;
        }
        self.methods.push(method);
        self
    }

    /// Adds a method that is always invoked with `.` (e.g. static constructors).
    pub fn static_method(mut self, method: Function) -> Self {
        self.methods.push(method.dot());
        self
    }

    pub fn get_field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    pub fn get_method(&self, name: &str) -> Option<&Function> {
        self.methods.iter().find(|m| m.name == name)
    }

    pub fn member(&self, name: &str) -> Option<Member<'_>> {
        self.get_method(name)
            .map(Member::Method)
            .or_else(|| self.get_field(name).map(Member::Field))
    }

    pub fn member_names(&self) -> Vec<&str> {
        self.methods
            .iter()
            .map(|m| m.name.as_str())
            .chain(self.fields.iter().map(|f| f.name.as_str()))
            .collect()
    }
}

/// A borrowed view of an [`ObjectType`] member.
#[derive(Debug, Clone, Copy)]
pub enum Member<'a> {
    Field(&'a Field),
    Method(&'a Function),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GlobalKind {
    Function(Function),
    /// A value of the given type: library tables, class factories, `Controls`, ...
    Value(TypeRef),
}

/// A name injected into the global scope of compiled programs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Global {
    pub name: String,
    pub doc: String,
    pub availability: Availability,
    pub kind: GlobalKind,
    /// Name of the plugin that registered this global.
    pub origin: String,
}

impl Global {
    pub fn function(function: Function) -> Self {
        Self {
            name: function.name.clone(),
            doc: function.doc.clone(),
            availability: Availability::Always,
            kind: GlobalKind::Function(function),
            origin: String::new(),
        }
    }

    pub fn value(name: impl Into<String>, ty: TypeRef) -> Self {
        Self {
            name: name.into(),
            doc: String::new(),
            availability: Availability::Always,
            kind: GlobalKind::Value(ty),
            origin: String::new(),
        }
    }

    /// A global bound to an object type of the same name (`Timer`, `math`, ...).
    pub fn object(name: impl Into<String>) -> Self {
        let name = name.into();
        Self::value(name.clone(), TypeRef::Object(name))
    }

    pub fn doc(mut self, doc: impl Into<String>) -> Self {
        self.doc = doc.into();
        self
    }

    pub fn available(mut self, availability: Availability) -> Self {
        self.availability = availability;
        self
    }

    pub fn runtime_only(self) -> Self {
        self.available(Availability::Runtime)
    }

    pub fn design_time_only(self) -> Self {
        self.available(Availability::DesignTime)
    }

    pub fn as_function(&self) -> Option<&Function> {
        match &self.kind {
            GlobalKind::Function(f) => Some(f),
            GlobalKind::Value(_) => None,
        }
    }

    /// The object type name this global is bound to, if any.
    pub fn object_type_name(&self) -> Option<&str> {
        match &self.kind {
            GlobalKind::Value(TypeRef::Object(name)) => Some(name),
            _ => None,
        }
    }
}

/// A name explicitly removed from the environment, with a user-facing reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Removed {
    pub name: String,
    pub reason: String,
    pub origin: String,
}
