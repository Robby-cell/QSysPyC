use std::sync::Arc;

use qsys_diagnostics::{Diagnostics, FileId, SourceFile};
use qsys_parser::parse_module;
use qsys_runtime::{
    EnvBuilder, Function, Global, LuaStdlib, PythonCore, QSysRuntime, RuntimeEnv, RuntimePlugin,
    TypeRef,
};
use qsys_sema::{SemanticModel, Type, analyze, analyze_with_env};

fn analyze_src(src: &str) -> Result<SemanticModel, Diagnostics> {
    let file = SourceFile::new(FileId(1), "test.py", src);
    analyze(&parse_module(file).expect("Should parse"))
}

fn analyze_src_with(src: &str, env: Arc<RuntimeEnv>) -> Result<SemanticModel, Diagnostics> {
    let file = SourceFile::new(FileId(1), "test.py", src);
    analyze_with_env(&parse_module(file).expect("Should parse"), env)
}

#[test]
fn controls_are_rejected_in_design_time_methods() {
    let src = r#"
class Mixer(QSysPlugin):
    def get_control_layout(self, props):
        x = Controls["Gain"]
        return {}
"#;
    let diags = analyze_src(src).expect_err("Controls is runtime-only");
    assert_eq!(diags.items.len(), 1);
    assert_eq!(diags.items[0].code(), "E0208");
    assert_eq!(
        diags.items[0].message(),
        "'Controls' is only available at runtime, but is used in design-time code"
    );
}

#[test]
fn controls_are_rejected_in_plugin_module_level_code() {
    let src = r#"
gain = Controls["Gain"]

class Mixer(QSysPlugin):
    pass
"#;
    let diags = analyze_src(src).expect_err("Module-level plugin code also runs at design time");
    assert_eq!(diags.items[0].code(), "E0208");
    assert!(
        diags.items[0]
            .message()
            .contains("both design time and runtime")
    );
}

#[test]
fn controls_are_allowed_in_runtime_code_and_deferred_functions() {
    let src = r#"
def read_gain():
    return Controls["Gain"].Value

class Helper:
    def poll(self):
        return Properties["Port"].Value

class Mixer(QSysPlugin):
    def runtime(self):
        Controls["Gain"].Value = 0.0
        t = Timer.New()
"#;
    analyze_src(src).expect("Runtime methods and function bodies may use runtime-only globals");
}

#[test]
fn controls_are_allowed_in_text_controller_module_code() {
    let src = r#"
gain = Controls["Gain"]

class Room(TextController):
    pass
"#;
    analyze_src(src).expect("Text controllers only execute at runtime");
}

#[test]
fn removed_builtins_report_the_plugin_reason() {
    let diags = analyze_src("x = eval(\"1 + 1\")\n").expect_err("eval is removed by PythonCore");
    assert_eq!(diags.items[0].code(), "E0209");
    assert_eq!(
        diags.items[0].message(),
        "'eval' is not available on this target: dynamic code execution cannot be compiled to Lua ahead of time"
    );

    let diags = analyze_src("name = input()\n").expect_err("input is removed by QSysRuntime");
    assert_eq!(diags.items[0].code(), "E0209");
    assert!(diags.items[0].message().contains("no standard input"));
}

#[test]
fn user_definitions_shadow_removed_builtins() {
    let src = r#"
def eval(x):
    return x

y = eval(1)
"#;
    analyze_src(src).expect("A user-defined `eval` is not the removed builtin");
}

#[test]
fn native_types_flow_from_the_runtime_env() {
    let src = r#"
t = Timer.New()
n = math.floor(2.5)
"#;
    let model = analyze_src(src).unwrap();
    let types: Vec<&Type> = model.type_map.types().values().collect();
    assert!(types.contains(&&Type::Native {
        name: "TimerHandle".to_string()
    }));
    assert!(types.contains(&&Type::Native {
        name: "Timer".to_string()
    }));
    assert!(types.contains(&&Type::Int), "math.floor returns int");
}

#[test]
fn annotations_naming_native_types_are_understood() {
    let src = r#"
def restart(t: TimerHandle):
    t.Start(1.0)
"#;
    let model = analyze_src(src).unwrap();
    assert!(model.type_map.types().values().any(|t| *t
        == Type::Native {
            name: "TimerHandle".to_string()
        }));
}

struct Telemetry;

impl RuntimePlugin for Telemetry {
    fn name(&self) -> &'static str {
        "telemetry"
    }

    fn install(&self, env: &mut EnvBuilder) {
        env.add_global(Global::function(
            Function::new("trace")
                .param("message", TypeRef::Str)
                .returns(TypeRef::Bool),
        ));
        env.remove("Timer", "timers are disabled on this target");
    }
}

#[test]
fn custom_plugins_can_add_and_remove_globals() {
    let env = Arc::new(
        RuntimeEnv::builder()
            .install(&PythonCore)
            .install(&LuaStdlib)
            .install(&QSysRuntime)
            .install(&Telemetry)
            .build(),
    );

    let model = analyze_src_with("ok = trace(\"hello\")\n", env.clone())
        .expect("trace() is provided by the Telemetry plugin");
    assert!(model.type_map.types().values().any(|t| *t == Type::Bool));

    let diags = analyze_src_with("t = Timer.New()\n", env).expect_err("Timer was removed");
    assert_eq!(diags.items[0].code(), "E0209");
    assert!(diags.items[0].message().contains("timers are disabled"));

    let diags = analyze_src("ok = trace(\"hello\")\n").expect_err("Not in the default env");
    assert_eq!(diags.items[0].code(), "E0206");
}
