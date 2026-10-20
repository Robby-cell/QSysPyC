use std::sync::{Arc, OnceLock};

use rustc_hash::FxHashMap;

use crate::defs::{CallStyle, Global, Member, ObjectType, Removed};
use crate::plugins::{LuaStdlib, PythonCore, QSysRuntime};

/// A bundle of runtime knowledge that can be installed into an [`EnvBuilder`].
///
/// ```
/// use qsys_runtime::{EnvBuilder, Function, Global, RuntimeEnv, RuntimePlugin, TypeRef};
///
/// struct Diagnostics;
///
/// impl RuntimePlugin for Diagnostics {
///     fn name(&self) -> &'static str {
///         "diagnostics"
///     }
///
///     fn install(&self, env: &mut EnvBuilder) {
///         env.add_global(Global::function(
///             Function::new("trace").param("message", TypeRef::Str),
///         ));
///         env.remove("print", "use trace() instead");
///     }
/// }
///
/// let env = RuntimeEnv::builder().install(&Diagnostics).build();
/// assert!(env.global("trace").is_some());
/// assert_eq!(env.removed_reason("print"), Some("use trace() instead"));
/// ```
pub trait RuntimePlugin {
    fn name(&self) -> &'static str;
    fn install(&self, env: &mut EnvBuilder);
}

/// Accumulates globals, object types and removals from a sequence of plugins.
///
/// Plugins are applied in order: a later plugin can override or [`remove`](Self::remove)
/// anything an earlier plugin registered.
#[derive(Debug, Default)]
pub struct EnvBuilder {
    globals: Vec<Global>,
    types: Vec<ObjectType>,
    removed: Vec<Removed>,
    plugins: Vec<String>,
    current_origin: String,
}

impl EnvBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies a plugin. Chainable for top-level environment construction.
    pub fn install(mut self, plugin: &dyn RuntimePlugin) -> Self {
        self.install_mut(plugin);
        self
    }

    /// Applies a plugin through a mutable reference (useful when composing plugins).
    pub fn install_mut(&mut self, plugin: &dyn RuntimePlugin) -> &mut Self {
        let previous = std::mem::replace(&mut self.current_origin, plugin.name().to_string());
        plugin.install(self);
        self.current_origin = previous;
        self.plugins.push(plugin.name().to_string());
        self
    }

    /// Registers (or replaces) a global. Re-adding a removed name makes it available again.
    pub fn add_global(&mut self, mut global: Global) -> &mut Self {
        global.origin.clone_from(&self.current_origin);
        self.removed.retain(|r| r.name != global.name);
        if let Some(existing) = self.globals.iter_mut().find(|g| g.name == global.name) {
            *existing = global;
        } else {
            self.globals.push(global);
        }
        self
    }

    /// Registers (or replaces) a native object type.
    pub fn add_type(&mut self, mut ty: ObjectType) -> &mut Self {
        ty.origin.clone_from(&self.current_origin);
        if let Some(existing) = self.types.iter_mut().find(|t| t.name == ty.name) {
            *existing = ty;
        } else {
            self.types.push(ty);
        }
        self
    }

    /// Removes a global and records why, so its use yields a targeted diagnostic.
    /// The name does not need to have been registered (e.g. unsupported Python builtins).
    pub fn remove(&mut self, name: impl Into<String>, reason: impl Into<String>) -> &mut Self {
        let name = name.into();
        self.globals.retain(|g| g.name != name);
        self.removed.retain(|r| r.name != name);
        self.removed.push(Removed {
            name,
            reason: reason.into(),
            origin: self.current_origin.clone(),
        });
        self
    }

    pub fn build(self) -> RuntimeEnv {
        let global_index = self
            .globals
            .iter()
            .enumerate()
            .map(|(i, g)| (g.name.clone(), i))
            .collect();
        let type_index = self
            .types
            .iter()
            .enumerate()
            .map(|(i, t)| (t.name.clone(), i))
            .collect();
        let removed = self
            .removed
            .iter()
            .map(|r| (r.name.clone(), r.clone()))
            .collect();

        RuntimeEnv {
            globals: self.globals,
            global_index,
            types: self.types,
            type_index,
            removed,
            plugins: self.plugins,
        }
    }
}

/// The immutable, queryable result of installing runtime plugins.
#[derive(Debug, Clone, Default)]
pub struct RuntimeEnv {
    globals: Vec<Global>,
    global_index: FxHashMap<String, usize>,
    types: Vec<ObjectType>,
    type_index: FxHashMap<String, usize>,
    removed: FxHashMap<String, Removed>,
    plugins: Vec<String>,
}

impl RuntimeEnv {
    pub fn builder() -> EnvBuilder {
        EnvBuilder::new()
    }

    /// The default Q-SYS environment: [`PythonCore`] + [`LuaStdlib`] + [`QSysRuntime`].
    /// Built once and shared.
    pub fn qsys() -> Arc<Self> {
        static ENV: OnceLock<Arc<RuntimeEnv>> = OnceLock::new();
        ENV.get_or_init(|| {
            Arc::new(
                Self::builder()
                    .install(&PythonCore)
                    .install(&LuaStdlib)
                    .install(&QSysRuntime)
                    .build(),
            )
        })
        .clone()
    }

    pub fn global(&self, name: &str) -> Option<&Global> {
        self.global_index.get(name).map(|&i| &self.globals[i])
    }

    /// All globals in registration order.
    pub fn globals(&self) -> &[Global] {
        &self.globals
    }

    pub fn global_names(&self) -> impl Iterator<Item = &str> {
        self.globals.iter().map(|g| g.name.as_str())
    }

    pub fn object_type(&self, name: &str) -> Option<&ObjectType> {
        self.type_index.get(name).map(|&i| &self.types[i])
    }

    /// All object types in registration order.
    pub fn object_types(&self) -> &[ObjectType] {
        &self.types
    }

    /// The object type a global is bound to (`Timer` -> `Timer`, `qsys` -> `QSysNamespace`).
    pub fn object_type_of_global(&self, name: &str) -> Option<&ObjectType> {
        self.global(name)
            .and_then(Global::object_type_name)
            .and_then(|t| self.object_type(t))
    }

    pub fn member(&self, type_name: &str, member: &str) -> Option<Member<'_>> {
        self.object_type(type_name).and_then(|t| t.member(member))
    }

    /// How `obj.member(...)` should be emitted for an object of `type_name`.
    pub fn call_style(&self, type_name: &str, member: &str) -> CallStyle {
        match self.object_type(type_name) {
            Some(ty) => ty
                .get_method(member)
                .map_or(ty.default_call_style, |m| m.call_style),
            None => CallStyle::Dot,
        }
    }

    pub fn removed_reason(&self, name: &str) -> Option<&str> {
        self.removed.get(name).map(|r| r.reason.as_str())
    }

    pub fn removed(&self) -> impl Iterator<Item = &Removed> {
        self.removed.values()
    }

    /// Names of the plugins installed, in order.
    pub fn plugins(&self) -> &[String] {
        &self.plugins
    }
}
