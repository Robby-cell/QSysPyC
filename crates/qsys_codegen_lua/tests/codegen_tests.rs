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
    assert!(lua.contains("(prefix .. val)"));
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

#[test]
fn test_global_custom_class_and_statements_order() {
    let plugin_src = r#"
class Foo:
    def __init__(self) -> None:
        pass

    def get_number(self) -> int:
        return 42

foo = Foo()
print(foo)

class MatrixMixer(QSysPlugin):
    name = "MatrixMixer"
    version = "1.0.0"
"#;
    let lua = compile_to_lua(plugin_src);

    let class_def_pos = lua
        .find("local Foo = {}")
        .expect("Must emit `local Foo = {}`");
    let ctor_def_pos = lua
        .find("function Foo.new()")
        .expect("Must emit `function Foo.new()`");
    let method_def_pos = lua
        .find("function Foo:get_number()")
        .expect("Must emit `function Foo:get_number()`");
    let inst_pos = lua
        .find("local foo = Foo.new()")
        .expect("Must emit `local foo = Foo.new()`");
    let print_pos = lua.find("print(foo)").expect("Must emit `print(foo)`");

    assert!(
        class_def_pos < ctor_def_pos
            && ctor_def_pos < method_def_pos
            && method_def_pos < inst_pos
            && inst_pos < print_pos,
        "Class definition and methods must be emitted BEFORE `local foo = Foo.new()` and `print(foo)`:\n{lua}"
    );
}

#[test]
fn test_custom_class_inheritance() {
    let plugin_src = r#"
class Foo:
    def __init__(self): pass
    def show(): print("Foo")

class Bar(Foo):
    def __init__(self): pass

bar = Bar()
bar.show()

class MatrixMixer(QSysPlugin):
    name = "MatrixMixer"
    version = "1.0.0"
"#;
    let lua = compile_to_lua(plugin_src);

    assert!(
        lua.contains("setmetatable(Bar, { __index = Foo })"),
        "Must link Bar metatable to Foo for inheritance:\n{lua}"
    );
    assert!(
        lua.contains("local bar = Bar.new()"),
        "Must instantiate Bar:\n{lua}"
    );
    assert!(
        lua.contains("bar:show()"),
        "Must dispatch inherited method with colon call:\n{lua}"
    );
}

#[test]
fn test_custom_class_inheritance_with_super() {
    let plugin_src = r#"
class Foo:
    def __init__(self, x: int):
        self.x = x
    def show(self):
        print(self.x)

class Bar(Foo):
    def __init__(self, x: int, y: int):
        super().__init__(x)
        self.y = y
    def show(self):
        super().show()
        print(self.y)

bar = Bar(10, 20)
bar.show()

class MatrixMixer(QSysPlugin):
    name = "MatrixMixer"
    version = "1.0.0"
"#;
    let lua = compile_to_lua(plugin_src);

    assert!(
        lua.contains("setmetatable(Bar, { __index = Foo })"),
        "Must link Bar metatable to Foo:\n{lua}"
    );
    assert!(
        lua.contains("function Foo:__init(x)"),
        "Base class must emit :__init when subclass calls super().__init__:\n{lua}"
    );
    assert!(
        lua.contains("Foo.__init(self, x)"),
        "Subclass super().__init__(x) must lower to Foo.__init(self, x):\n{lua}"
    );
    assert!(
        lua.contains("Foo.show(self)"),
        "Subclass super().show() must lower to Foo.show(self):\n{lua}"
    );
}

#[test]
fn test_runtime_env_call_styles() {
    let src = r#"
t = Timer.New()
t.Start(0.5)
sock = UdpSocket.New()
sock.Send("255.255.255.255", 9, "hi")
n = math.floor(2.5)
qsys.lua("print('raw')")
"#;
    let lua = compile_to_lua(src);

    assert!(
        lua.contains("Timer.New()"),
        "Factories use dot calls:\n{lua}"
    );
    assert!(
        lua.contains("t:Start(0.5)"),
        "Handle methods use colon calls:\n{lua}"
    );
    assert!(
        lua.contains("UdpSocket.New()"),
        "Factories use dot calls:\n{lua}"
    );
    assert!(
        lua.contains("sock:Send(\"255.255.255.255\", 9, \"hi\")"),
        "Handle methods use colon calls:\n{lua}"
    );
    assert!(
        lua.contains("math.floor(2.5)"),
        "Library functions use dot calls:\n{lua}"
    );
    assert!(
        lua.contains("print('raw')"),
        "Escape hatch emits raw Lua:\n{lua}"
    );
}

#[test]
fn test_runtime_env_builtin_lowering() {
    let src = r#"
items = [1, 2, 3]
n = len(items)
s = str(n)
m = abs(-3)
c = chr(65)
"#;
    let lua = compile_to_lua(src);

    assert!(
        lua.contains("#items"),
        "len() lowers to the # operator:\n{lua}"
    );
    assert!(
        lua.contains("tostring(n)"),
        "str() lowers to tostring():\n{lua}"
    );
    assert!(
        lua.contains("math.abs("),
        "abs() lowers to math.abs():\n{lua}"
    );
    assert!(
        lua.contains("string.char(65)"),
        "chr() lowers to string.char():\n{lua}"
    );
}

#[test]
fn test_user_functions_shadow_builtin_lowering() {
    let src = r#"
def len(x):
    return 42

items = [1, 2, 3]
n = len(items)
"#;
    let lua = compile_to_lua(src);

    assert!(
        !lua.contains("#items"),
        "A user-defined len() must not be lowered to the # operator:\n{lua}"
    );
    assert!(
        lua.contains("len(items)"),
        "User len() is called as written:\n{lua}"
    );
}
