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

#[test]
fn test_wake_on_lan_example_compiles_cleanly() {
    let src = include_str!("../../../examples/WakeOnLAN/plugin.py");
    let options = CompilerOptions {
        filename: "plugin.py".to_string(),
        ..Default::default()
    };

    let artifact = Compiler::compile(src, &options).expect("WakeOnLAN must compile without errors");
    assert_eq!(artifact.program_kind, ProgramKind::Plugin);
    assert_eq!(artifact.control_count, 5);

    // Ensure proper Q-SYS Lua 5.3 emission
    assert!(artifact.lua.contains("local sock = UdpSocket.New()"));
    assert!(artifact.lua.contains("sock:Open()"));
    assert!(artifact.lua.contains("string.rep(\"\\xff\", 6)"));
    assert!(
        artifact
            .lua
            .contains("sock:Send(Controls[\"IPAddress\"].String, 9, packet)")
    );
    assert!(artifact.lua.contains("sock:Close()"));
    assert!(
        artifact
            .lua
            .contains("Controls[\"Status\"].String = \"Packet sent\"")
    );
    assert!(artifact.lua.contains("WakeOnLan_send_packet(nil)"));
    assert!(!artifact.lua.contains("-- Plugin initialization"));
}

#[test]
fn test_fault_tolerant_completions_and_signature_help_mid_edit() {
    let incomplete_self = r#"
class Mixer(QSysPlugin):
    MasterGain = Control.Float(name="MasterGain", min=-100.0, max=20.0)
    MuteButton = Control.Button(name="Mute", type="Toggle")
    Port = Property.Integer(name="Port", default=9)

    def runtime(self):
        self.
"#;
    // Cursor right after `self.` on line 8, col 14 — file has a syntax error!
    let completions = Analyzer::completions(incomplete_self, 8, 14);
    assert!(
        completions.iter().any(|c| c.label == "MasterGain"),
        "Must suggest MasterGain even when `self.` has a trailing dot syntax error"
    );
    assert!(completions.iter().any(|c| c.label == "MuteButton"));
    assert!(completions.iter().any(|c| c.label == "Port"));

    // Partial prefix filtering after `self.Ma`
    let incomplete_partial = r#"
class Mixer(QSysPlugin):
    MasterGain = Control.Float(name="MasterGain", min=-100.0, max=20.0)
    MuteButton = Control.Button(name="Mute", type="Toggle")

    def runtime(self):
        self.Ma
"#;
    let filtered = Analyzer::completions(incomplete_partial, 7, 16);
    assert!(filtered.iter().any(|c| c.label == "MasterGain"));
    assert!(!filtered.iter().any(|c| c.label == "MuteButton"));

    // Socket method completions after `sock.`
    let socket_src = r#"
class Wol(QSysPlugin):
    def send(self):
        sock = UdpSocket.New()
        sock.
"#;
    let sock_completions = Analyzer::completions(socket_src, 5, 14);
    assert!(sock_completions.iter().any(|c| c.label == "Send"));
    assert!(sock_completions.iter().any(|c| c.label == "Open"));
    assert!(sock_completions.iter().any(|c| c.label == "Close"));

    // Signature help inside `sock.Send(ip, `
    let sig_src = r#"
class Wol(QSysPlugin):
    def send(self):
        sock = UdpSocket.New()
        sock.Send("255.255.255.255", 
"#;
    let sig = Analyzer::signature_help(sig_src, 5, 38).expect("Must provide signature help");
    assert!(sig.label.contains("sock.Send"));
    assert_eq!(sig.active_parameter, 1);

    // Inlay hints for untyped event handler `ctl` parameter and `sock` local
    let hint_src = r#"
class Wol(QSysPlugin):
    Btn = Control.Button(name="Btn", type="Trigger")

    @Btn.EventHandler
    def on_btn(self, ctl):
        sock = UdpSocket.New()
"#;
    let hints = Analyzer::inlay_hints(hint_src);
    assert!(
        hints
            .iter()
            .any(|h| h.line == 6 && h.col == 25 && h.label == ": ControlHandle")
    );
    assert!(
        hints
            .iter()
            .any(|h| h.line == 7 && h.col == 13 && h.label == ": UdpSocketHandle")
    );
}

#[test]
fn test_runtime_env_powers_ide_features() {
    // Completions on a runtime global come from its object type in the RuntimeEnv
    let timer_src = "t = Timer.\n";
    let timer_items = Analyzer::completions(timer_src, 1, 11);
    assert!(timer_items.iter().any(|c| c.label == "New"));
    assert!(timer_items.iter().any(|c| c.label == "CallAfter"));

    // Members of a handle returned by a factory method
    let handle_src = "t = Timer.New()\nt.\n";
    let handle_items = Analyzer::completions(handle_src, 2, 3);
    assert!(
        handle_items
            .iter()
            .any(|c| c.label == "Start" && c.kind == "Method")
    );
    assert!(
        handle_items
            .iter()
            .any(|c| c.label == "EventHandler" && c.kind == "Event")
    );

    // Library tables registered by LuaStdlib
    let math_items = Analyzer::completions("x = math.fl\n", 1, 12);
    assert!(math_items.iter().any(|c| c.label == "floor"));
    assert!(!math_items.iter().any(|c| c.label == "ceil"));

    // Top-level completions include runtime globals
    let top_items = Analyzer::completions("Tcp\n", 1, 4);
    assert!(
        top_items
            .iter()
            .any(|c| c.label == "TcpSocket" && c.kind == "Class")
    );

    // Hover on a runtime global and on a runtime-only global
    let hover = Analyzer::hover("t = Timer.New()\n", 1, 6).expect("Hover for Timer");
    assert_eq!(hover.contents[0], "*(class)* Timer");
    let hover = Analyzer::hover(
        "class P(QSysPlugin):\n    def runtime(self):\n        Controls[\"x\"].Value = 1\n",
        3,
        10,
    )
    .expect("Hover for Controls");
    assert!(hover.contents[0].contains("Controls: dict[str, ControlHandle]"));
    assert!(hover.contents[1].contains("Only available at runtime"));

    // Hover on a native member resolves through the receiver type
    let hover = Analyzer::hover("t = Timer.New()\nt.Start(1.0)\n", 2, 4).expect("Hover for Start");
    assert!(hover.contents[0].contains("TimerHandle.Start(seconds: float) -> None"));

    // Signature help for a library function and for a Python builtin
    let sig = Analyzer::signature_help("x = math.floor(", 1, 16).expect("Signature for math.floor");
    assert_eq!(sig.label, "math.floor(x: float)");
    let sig = Analyzer::signature_help("x = len(", 1, 9).expect("Signature for len");
    assert_eq!(sig.label, "len(obj: Any)");
}

#[test]
fn test_declarative_ui_diagnostics_and_preview_payload() {
    // 1. Typo in UI.Field control reference produces fuzzy UnknownClassMember suggestion
    let bad_src = r#"
class Mixer(QSysPlugin):
    MasterGain = Control.Float(name="MasterGain", min=-100.0, max=20.0)
    layout = UI.Page("Main", UI.Field("Gain", MasterGani))
"#;
    let bad_errs = Compiler::compile(bad_src, &CompilerOptions::default()).unwrap_err();
    let msg = bad_errs.items[0].message();
    assert!(
        msg.contains("MasterGani") && msg.contains("did you mean 'MasterGain'?"),
        "Should suggest MasterGain for typo in UI.Field: {msg}"
    );

    // 2. Out-of-bounds channel subscript in UI.Control produces error
    let oob_src = r#"
class Mixer(QSysPlugin):
    Ch = Control.Float(name="Ch", count=4)
    layout = UI.Page("Main", UI.Control(Ch[4]))
"#;
    let oob_errs = Compiler::compile(oob_src, &CompilerOptions::default()).unwrap_err();
    assert!(
        oob_errs.items[0].message().contains("out of bounds"),
        "Should catch out-of-bounds channel index: {}",
        oob_errs.items[0].message()
    );

    // 3. Valid plugin produces UI preview JSON payload with pages, controls, and graphics
    let ok_src = r#"
class Mixer(QSysPlugin):
    Master = Control.Float(name="MasterGain", min=-100.0, max=20.0, default=0.0, unit="dB")
    Mute = Control.Button(name="MuteAll", type="Toggle")

    layout = UI.Page(
        "Main",
        UI.GroupBox("Master",
            UI.Row(
                UI.Field("Master", Master, style="Fader"),
                UI.Field("Mute", Mute),
            ),
        ),
    )
"#;
    let ok_artifact = Compiler::compile(ok_src, &CompilerOptions::default()).expect("Should compile");
    let preview_json = ok_artifact.to_ui_preview_json();
    assert!(preview_json.contains("\"has_explicit_layout\":true"));
    assert!(preview_json.contains("\"MasterGain\""));
    assert!(preview_json.contains("\"MuteAll\""));

    // 4. Completions after `UI.` include Page, GroupBox, Row, Col, Field, Control
    let ui_completions = Analyzer::completions("class M(QSysPlugin):\n    layout = UI.\n", 2, 17);
    assert!(ui_completions.iter().any(|c| c.label == "Page"));
    assert!(ui_completions.iter().any(|c| c.label == "GroupBox"));
    assert!(ui_completions.iter().any(|c| c.label == "Field"));
}

