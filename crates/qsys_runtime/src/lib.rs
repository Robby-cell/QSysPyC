//! Declarative description of the runtime a QSysPy program is compiled against.
//!
//! A [`RuntimeEnv`] is assembled from [`RuntimePlugin`]s. Each plugin can add globals
//! (functions and values), native object types (fields, methods, events) and remove names
//! with a user-facing reason. Semantic analysis, Lua lowering, IDE features and `.pyi`
//! stub generation all read from the same environment, so they cannot drift apart.
//!
//! The default Q-SYS environment is [`RuntimeEnv::qsys`]:
//! [`PythonCore`] + [`LuaStdlib`] + [`QSysRuntime`].

mod defs;
mod env;
pub mod plugins;
pub mod pyi;

pub use crate::defs::{
    Availability, CallStyle, ExecutionContext, Field, Function, Global, GlobalKind, Lowering,
    Member, ObjectType, Param, Removed, Returns, TypeRef,
};
pub use crate::env::{EnvBuilder, RuntimeEnv, RuntimePlugin};
pub use crate::plugins::{LuaStdlib, PythonCore, QSysRuntime};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_env_contains_all_plugins() {
        let env = RuntimeEnv::qsys();
        assert_eq!(
            env.plugins(),
            [PythonCore::NAME, LuaStdlib::NAME, QSysRuntime::NAME]
        );
        assert!(env.global("print").is_some());
        assert!(env.global("math").is_some());
        assert!(env.global("Timer").is_some());
    }

    #[test]
    fn controls_and_properties_are_runtime_only() {
        let env = RuntimeEnv::qsys();
        for name in ["Controls", "Properties"] {
            let global = env.global(name).unwrap();
            assert_eq!(global.availability, Availability::Runtime);
            assert!(!global.availability.permits(ExecutionContext::DesignTime));
            assert!(!global.availability.permits(ExecutionContext::Both));
            assert!(global.availability.permits(ExecutionContext::Runtime));
        }
        assert!(
            env.global("Timer")
                .unwrap()
                .availability
                .permits(ExecutionContext::DesignTime)
        );
    }

    #[test]
    fn handle_methods_use_colon_and_factories_use_dot() {
        let env = RuntimeEnv::qsys();
        assert_eq!(env.call_style("TimerHandle", "Start"), CallStyle::Colon);
        assert_eq!(
            env.call_style("TimerHandle", "Undeclared"),
            CallStyle::Colon
        );
        assert_eq!(env.call_style("Timer", "New"), CallStyle::Dot);
        assert_eq!(env.call_style("math", "floor"), CallStyle::Dot);
    }

    #[test]
    fn later_plugins_override_and_remove() {
        struct Base;
        impl RuntimePlugin for Base {
            fn name(&self) -> &'static str {
                "base"
            }
            fn install(&self, env: &mut EnvBuilder) {
                env.add_global(Global::function(Function::new("helper")));
                env.add_global(Global::function(Function::new("legacy")));
            }
        }

        struct Restrict;
        impl RuntimePlugin for Restrict {
            fn name(&self) -> &'static str {
                "restrict"
            }
            fn install(&self, env: &mut EnvBuilder) {
                env.remove("legacy", "removed on this target");
                env.add_global(
                    Global::function(Function::new("helper").returns(TypeRef::Int)).runtime_only(),
                );
            }
        }

        let env = RuntimeEnv::builder()
            .install(&Base)
            .install(&Restrict)
            .build();
        assert!(env.global("legacy").is_none());
        assert_eq!(env.removed_reason("legacy"), Some("removed on this target"));

        let helper = env.global("helper").unwrap();
        assert_eq!(helper.origin, "restrict");
        assert_eq!(helper.availability, Availability::Runtime);
    }

    #[test]
    fn re_adding_a_removed_name_restores_it() {
        let mut builder = RuntimeEnv::builder();
        builder.remove("print", "no console");
        builder.add_global(Global::function(Function::new("print")));
        let env = builder.build();
        assert!(env.global("print").is_some());
        assert!(env.removed_reason("print").is_none());
    }

    #[test]
    fn pyi_renders_qsys_types_only() {
        let env = RuntimeEnv::qsys();
        let pyi = pyi::render_pyi(&env, |origin| origin == QSysRuntime::NAME);

        assert!(pyi.contains("class TimerHandle:"));
        assert!(pyi.contains("    def Start(self, seconds: float) -> None:"));
        assert!(pyi.contains("class Timer:\n"));
        assert!(pyi.contains("    @staticmethod\n    def New() -> TimerHandle:"));
        assert!(pyi.contains("    EventHandler: Callable[[TimerHandle], None]"));
        assert!(pyi.contains("Controls: dict[str, ControlHandle]"));
        assert!(pyi.contains("qsys: QSysNamespace"));
        assert!(
            !pyi.contains("class math:"),
            "Lua stdlib must be filtered out"
        );
        assert!(
            !pyi.contains("def print("),
            "Python builtins must be filtered out"
        );
    }
}
