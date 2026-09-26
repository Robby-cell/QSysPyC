use qsys_compiler::{Analyzer, Compiler, CompilerOptions, ProgramKind};

#[test]
fn test_compiler_powered_hover_and_completions() {
    let src = r#"
class Mixer(QSysPlugin):
    Master = Control.Float(name="MasterGain", min=-100.0, max=20.0)

    @Master.EventHandler
    def on_gain(self, ctl):
        self.Master.Value = 0.0
"#;

    // 1. Verify completions after `self.` return the Rust declared control
    let completions = Analyzer::completions(src, 7, 14); // after `self.`
    assert!(completions.iter().any(|c| c.label == "Master"));

    // 2. Verify hover over `Value` returns docstring from qsys_model
    let hover = Analyzer::hover(src, 7, 22).expect("Must return hover for Value");
    assert!(hover.contents[0].contains("ControlHandle.Value: float"));
}

#[test]
fn test_compiler_driver_plugin_pipeline() {
    let src = r#"
class GainController(QSysPlugin):
    Master = Control.Float(name="MasterGain", min=-100.0, max=20.0, default=0.0)

    @Master.EventHandler
    def on_gain(self, ctl):
        pass
"#;
    let options = CompilerOptions {
        filename: "gain.py".to_string(),
        ..Default::default()
    };

    let artifact = Compiler::compile(src, &options).expect("Compilation must succeed");

    assert_eq!(artifact.program_kind, ProgramKind::Plugin);
    assert_eq!(artifact.control_count, 1);
    assert_eq!(artifact.property_count, 0);
    assert_eq!(
        artifact.plugin_info.as_ref().unwrap().name,
        "GainController"
    );
    assert!(artifact.lua.contains("PluginInfo = {"));
    assert!(
        artifact
            .lua
            .contains("Controls[\"MasterGain\"].EventHandler")
    );
}

#[test]
fn test_compiler_driver_check_mode() {
    let src = "print('Hello Q-SYS')";
    let check = Compiler::check(src, "script.py").expect("Check must succeed");
    assert_eq!(check.program_kind, ProgramKind::ProceduralScript);
}

#[test]
fn test_compiler_driver_catches_errors_with_markers() {
    // Member typo error
    let bad_src = r#"
class BadPlugin(QSysPlugin):
    Gain = Control.Float()

    @Gain.EventHandler
    def on_gain(self, ctl):
        val = ctl.Valu
"#;
    let options = CompilerOptions::default();
    let errs = Compiler::compile(bad_src, &options).unwrap_err();

    assert_eq!(errs.items.len(), 1);
    assert_eq!(errs.items[0].code(), "E0201");

    let file = qsys_diagnostics::SourceFile::new(qsys_diagnostics::FileId(1), "bad.py", bad_src);
    let markers = errs.to_editor_markers(&file);
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].code, "E0201");
    assert!(markers[0].message.contains("did you mean 'Value'?"));
}
