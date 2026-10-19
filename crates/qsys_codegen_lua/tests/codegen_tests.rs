use qsys_codegen_lua::LuaBackend;
use qsys_diagnostics::{FileId, SourceFile};
use qsys_ir::{Backend, CodegenConfig};
use qsys_parser::parse_module;
use qsys_sema::analyze;

fn compile_to_lua(src: &str) -> String {
    let file = SourceFile::new(FileId(1), "test.py", src);
    let parsed = parse_module(file).expect("Should parse");
    let model = analyze(&parsed).expect("Should analyze");
    let ir = model.into_ir();

    let backend = LuaBackend::new();
    let config = CodegenConfig::default();
    let (lua, diags) = backend.emit(&ir, &config).expect("Codegen should succeed");
    assert!(!diags.has_errors());
    lua
}

#[test]
fn test_plugin_full_lifecycle_generation() {
    let src = r#"
class MatrixMixer(QSysPlugin):
    Master = Control.Float(name="MasterGain", min=-100.0, max=20.0, default=0.0)
    Mute = Control.Button(name="MuteAll", type="Toggle")
    Channels = Property.Integer(name="ChannelCount", default=8, min=1, max=64)

    @Master.EventHandler
    def on_gain(self, ctl):
        if ctl.Value:
            self.Mute.Boolean = False
"#;
    let lua = compile_to_lua(src);

    // Verify PluginInfo
    assert!(lua.contains("PluginInfo = {"));
    assert!(lua.contains("Name = \"MatrixMixer\""));

    // Verify GetProperties
    assert!(lua.contains("function GetProperties()"));
    assert!(lua.contains("Name = \"ChannelCount\""));
    assert!(lua.contains("Type = \"integer\""));
    assert!(lua.contains("Value = 8"));

    // Verify GetControls
    assert!(lua.contains("function GetControls(props)"));
    assert!(lua.contains("Name = \"MasterGain\""));
    assert!(lua.contains("ControlType = \"Knob\""));
    assert!(lua.contains("Name = \"MuteAll\""));
    assert!(lua.contains("ButtonType = \"Toggle\""));

    // Verify Runtime Guard & Event Handler Binding
    assert!(lua.contains("if Controls then"));
    assert!(lua.contains("Controls[\"MasterGain\"].EventHandler = function(ctl)"));
    assert!(lua.contains("MatrixMixer_on_gain(nil, ctl)"));

    // Verify Truthiness numeric guard: ctl.Value != 0
    assert!(lua.contains("(ctl.Value ~= 0)"));

    // Verify hardware control assignment
    assert!(lua.contains("Controls[\"MuteAll\"].Boolean = false"));
}

#[test]
fn test_string_concatenation_and_indexing() {
    let src = r#"
def format_label(prefix: str, idx: int):
    items = ["Alpha", "Beta"]
    val = items[idx]
    res = prefix + val
"#;
    let lua = compile_to_lua(src);

    // Verify function header
    assert!(lua.contains("local function format_label(prefix, idx)"));

    // Verify list literal
    assert!(lua.contains("{ \"Alpha\", \"Beta\" }"));

    // Verify 1-based indexing offset: [(idx) + 1]
    assert!(lua.contains("[(idx) + 1]"));

    // Verify string concatenation: (prefix .. val)
    assert!(lua.contains("(prefix .. _v"));
}

#[test]
fn test_python_truthiness_unary_not() {
    let src = r#"
def check_level(level: float, name: str):
    is_silent = not level
    is_anonymous = not name
"#;
    let lua = compile_to_lua(src);

    // In Python: `not float` is (level == 0)
    assert!(lua.contains("(level == 0)"));

    // In Python: `not str` is (#name == 0)
    assert!(lua.contains("(#name == 0)"));
}

#[test]
fn test_escape_hatch_raw_lua() {
    let src = r#"
def run():
    qsys.lua("Component.New('MyDSP')")
"#;
    let lua = compile_to_lua(src);
    assert!(lua.contains("Component.New('MyDSP')"));
}

#[test]
fn test_python_builtins_fstrings_range_and_tuple_returns() {
    let src = r#"
class LayoutPlugin(QSysPlugin):
    Status = Control.Text(name="Status", default="Ready")
    Port = Property.Integer(name="Port", default=9)

    def get_control_layout(self, props):
        layout = {}
        graphics = []
        return layout, graphics

    def runtime(self):
        port = self.Port.Value
        for i in range(0, 4):
            if i == 2:
                break
        msg = f"Listening on {port}"
        n = len(msg)
        b = chr(65)
        self.Status.String = msg
"#;
    let lua = compile_to_lua(src);

    // Multiple return values in GetControlLayout
    assert!(lua.contains("function GetControlLayout(props)"));
    assert!(lua.contains("return layout, graphics"));

    // Property access without double .Value.Value
    assert!(lua.contains("local port = Properties[\"Port\"].Value"));
    assert!(!lua.contains(".Value.Value"));

    // Numeric for loop from range(0, 4) and break
    assert!(lua.contains("for i = 0, (4) - 1 do"));
    assert!(lua.contains("break"));

    // f-string lowered with tostring() and ..
    assert!(lua.contains("(\"Listening on \" .. tostring(port))"));

    // len() and chr() lowered to # and string.char
    assert!(lua.contains("(#msg)"));
    assert!(lua.contains("string.char(65)"));
}
