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

#[test]
fn test_declarative_ui_layout_direct_control_refs_and_multichannel() {
    let src = r##"
class MatrixMixer(QSysPlugin):
    Gain = Control.Float(name="MasterGain", min=-100.0, max=20.0, default=0.0)
    Mute = Control.Button(name="MuteAll", type="Toggle")
    ChGain = Control.Float(name="Ch", count=3, min=-60.0, max=12.0, default=0.0)
    Status = Control.Text(name="Status", default="Ready", type="ReadOnly")

    layout = UI.Page(
        "Mixer",
        UI.Header("8x8 Matrix Gain Processor"),
        UI.GroupBox("Master Section",
            UI.Row(
                UI.Field("Master Gain", Gain, style="Fader", size=(50, 120)),
                UI.Field("Mute All", self.Mute, color="#ef4444"),
                UI.Field("Status", Status, width=140),
            ),
        ),
        UI.GroupBox("Channel Strips",
            UI.Row(
                UI.Control(ChGain, style="Knob"),
                UI.Field("Ch 1 Direct", ChGain[0], style="Text"),
            ),
        ),
    )
"##;
    let lua = compile_to_lua(src);

    assert!(
        lua.contains("function GetControlLayout(props)"),
        "Must emit GetControlLayout:\n{lua}"
    );
    assert!(
        lua.contains("Type = \"Header\"") && lua.contains("Text = \"8x8 Matrix Gain Processor\""),
        "Must emit Header graphic:\n{lua}"
    );
    assert!(
        lua.contains("Type = \"GroupBox\"") && lua.contains("Text = \"Master Section\""),
        "Must emit GroupBox graphic:\n{lua}"
    );
    assert!(
        lua.contains("layout[\"MasterGain\"] = {\n        PrettyName = \"Master Gain\",\n        Style = \"Fader\"")
            && lua.contains("Size = { 50, 120 }"),
        "Direct control reference Gain must resolve to its Q-SYS control name 'MasterGain' with custom size:\n{lua}"
    );
    assert!(
        lua.contains("layout[\"MuteAll\"] = {\n        PrettyName = \"Mute All\",\n        Style = \"Button\""),
        "self.Mute must resolve to 'MuteAll' with Button style:\n{lua}"
    );
    assert!(
        lua.contains("Color = { 239, 68, 68 }"),
        "Hex color #ef4444 must convert to RGB tuple:\n{lua}"
    );
    // Multi-channel control ChGain (count=3) referenced bare in UI.Control(ChGain) expands to Ch 1, Ch 2, Ch 3 with PrettyNames
    assert!(
        lua.contains(
            "layout[\"Ch 2\"] = {\n        PrettyName = \"Ch 2\",\n        Style = \"Knob\""
        ),
        "Multi-channel control must expand to 'Ch 1', 'Ch 2', 'Ch 3' with PrettyName:\n{lua}"
    );
    assert!(
        lua.contains(
            "layout[\"Ch 3\"] = {\n        PrettyName = \"Ch 3\",\n        Style = \"Knob\""
        ),
        "Multi-channel control must expand to 'Ch 3' with PrettyName:\n{lua}"
    );
}

#[test]
fn test_declarative_multi_page_ui_layout() {
    let src = r#"
class MultiPagePlugin(QSysPlugin):
    IPAddress = Control.IPAddress(name="IP", default="192.168.1.100")
    Gain = Control.Float(name="Gain", min=-100.0, max=20.0, default=0.0)

    pages = [
        UI.Page("Setup",
            UI.GroupBox("Connection",
                UI.Field("Host IP", IPAddress),
            ),
        ),
        UI.Page("Audio",
            UI.GroupBox("Levels",
                UI.Field("Output Gain", Gain, style="Fader"),
            ),
        ),
    ]
"#;
    let lua = compile_to_lua(src);

    assert!(
        lua.contains("local PageNames = { \"Setup\", \"Audio\" }"),
        "Must emit PageNames table:\n{lua}"
    );
    assert!(
        lua.contains("function GetPages(props)"),
        "Must emit GetPages(props):\n{lua}"
    );
    assert!(
        lua.contains("if CurrentPage == \"Setup\" then"),
        "Must emit Setup page branch:\n{lua}"
    );
    assert!(
        lua.contains("elseif CurrentPage == \"Audio\" then"),
        "Must emit Audio page branch:\n{lua}"
    );
    assert!(
        lua.contains("layout[\"IP\"] = {\n            PrettyName = \"Host IP\",\n            Style = \"Text\""),
        "Setup page must emit IP control:\n{lua}"
    );
    assert!(
        lua.contains("layout[\"Gain\"] = {\n            PrettyName = \"Output Gain\",\n            Style = \"Fader\""),
        "Audio page must emit Gain control:\n{lua}"
    );
}

#[test]
fn test_declarative_get_control_layout_method_return() {
    let src = r#"
class MethodLayoutPlugin(QSysPlugin):
    Power = Control.Button(name="SystemPower", type="Toggle")

    def get_control_layout(self):
        return UI.Page(
            "Main",
            UI.Field("System Power", Power),
        )
"#;
    let lua = compile_to_lua(src);

    assert!(
        lua.contains("layout[\"SystemPower\"] = {\n        PrettyName = \"System Power\",\n        Style = \"Button\""),
        "Declarative return from get_control_layout(self) must solve layout:\n{lua}"
    );
}

#[test]
fn test_get_control_layout_with_layout_assignment_and_return_tuple() {
    let src = r#"
class MyPlugin(QSysPlugin):
    Gain = Control.Float(name="MasterGain", min=-100.0, max=20.0, default=0.0)

    def get_control_layout(self, props):
        layout = {}
        graphics = []
        layout = UI.Page(
            "Main",
            UI.Header("Plugin Title"),
            UI.GroupBox(
                "Controls",
                UI.Field("Master Gain", self.Gain, style="Fader"),
            ),
        )
        return layout, graphics
"#;
    let lua = compile_to_lua(src);

    assert!(
        !lua.contains("UI.Page"),
        "UI.Page must never be emitted raw into Lua:\n{lua}"
    );
    assert!(
        lua.contains("Type = \"Header\"") && lua.contains("Text = \"Plugin Title\""),
        "Must expand UI.Header into graphics table:\n{lua}"
    );
    assert!(
        lua.contains("Type = \"GroupBox\"") && lua.contains("Text = \"Controls\""),
        "Must expand UI.GroupBox into graphics table:\n{lua}"
    );
    assert!(
        lua.contains("layout[\"MasterGain\"] = {"),
        "Must expand UI.Field into layout table:\n{lua}"
    );
}

#[test]
fn test_hybrid_get_control_layout_with_if_branches() {
    let src = r#"
class HybridPlugin(QSysPlugin):
    Gain = Control.Float(name="MasterGain", min=-100.0, max=20.0, default=0.0)

    def get_control_layout(self, props):
        layout = {}
        graphics = []
        if props["Simple"].Value:
            layout = UI.Page("Main", UI.Field("Gain", self.Gain))
        else:
            layout = UI.Page("Main", UI.Header("Full Mode"), UI.Field("Gain", self.Gain))
        return layout, graphics
"#;
    let lua = compile_to_lua(src);

    assert!(
        !lua.contains("UI.Page"),
        "UI.Page inside if/else in get_control_layout must expand into Q-SYS tables:\n{lua}"
    );
    assert!(
        lua.contains("Text = \"Full Mode\""),
        "Else branch UI.Header must expand into graphics table:\n{lua}"
    );
}

#[test]
fn test_multichannel_count2_pretty_names_comprehensions_and_runtime_events() {
    let src = r#"
class StereoPlugin(QSysPlugin):
    Gain = Control.Float(name="Gain", min=-100.0, max=20.0, default=0.0, count=2)
    Mute = Control.Button(name="Mute", type="Toggle", count=2)

    def get_control_layout(self, props):
        layout = UI.Page(
            "Main",
            UI.Header("Stereo Channel Strip"),
            UI.GroupBox(
                "Gains",
                UI.Field("Gain", self.Gain, labels=["Left Gain", "Right Gain"], direction="row"),
            ),
            UI.GroupBox(
                "Mutes",
                [UI.Field(f"Ch {i + 1} Mute", self.Mute[i]) for i in range(2)],
            ),
        )
        return layout

    @Gain.EventHandler
    def on_gain(self, ctl, index):
        self.Mute[index].Boolean = ctl.Value < -80.0

    @Mute[0].EventHandler
    def on_left_mute(self, ctl):
        self.Gain[0].Value = -100.0
"#;
    let lua = compile_to_lua(src);

    assert!(
        lua.contains("layout[\"Gain 1\"] = {\n        PrettyName = \"Left Gain\""),
        "Gain 1 must have PrettyName = 'Left Gain':\n{lua}"
    );
    assert!(
        lua.contains("layout[\"Gain 2\"] = {\n        PrettyName = \"Right Gain\""),
        "Gain 2 must have PrettyName = 'Right Gain':\n{lua}"
    );
    assert!(
        lua.contains("layout[\"Mute 1\"] = {\n        PrettyName = \"Ch 1 Mute\""),
        "Comprehension must expand Mute[0] to Mute 1 with PrettyName = 'Ch 1 Mute':\n{lua}"
    );
    assert!(
        lua.contains("layout[\"Mute 2\"] = {\n        PrettyName = \"Ch 2 Mute\""),
        "Comprehension must expand Mute[1] to Mute 2 with PrettyName = 'Ch 2 Mute':\n{lua}"
    );
    assert!(
        lua.contains("for ix, ctl in ipairs(Controls[\"Gain\"]) do"),
        "@Gain.EventHandler on count=2 control must iterate Controls['Gain']:\n{lua}"
    );
    assert!(
        lua.contains("Controls[\"Mute\"][1].EventHandler = function(ctl)"),
        "@Mute[0].EventHandler must bind 1-based Controls['Mute'][1]:\n{lua}"
    );
    assert!(
        lua.contains("Controls[\"Gain\"][1].Value = (-100.0)"),
        "self.Gain[0].Value must lower to 1-based Controls['Gain'][1].Value:\n{lua}"
    );
}

#[test]
fn test_property_driven_control_count_handles_1_vs_2_plus() {
    let src = r#"
class DynamicInputPlugin(QSysPlugin):
    NumInputs = Property.Integer("number of inputs", default=4, min=1, max=16)

    Inputs = Control.Fader(
        name="inputs",
        min=-100.0,
        max=12.0,
        default=0.0,
        unit="dB",
        count=NumInputs,
    )
    Mutes = Control.Boolean(
        name="mutes",
        type="Toggle",
        count=props["number of inputs"].Value,
    )

    layout = UI.Page(
        "Main",
        UI.GroupBox(
            "Input Strip",
            UI.Field("Input {i}", Inputs, direction="row"),
            UI.Field("Ch 1 Mute", Mutes[0]),
            UI.Field("Ch 2 Mute", Mutes[1]),
        ),
    )

    @Inputs.EventHandler
    def on_input(self, ctl, index):
        self.Mutes[index].Boolean = ctl.Value <= -100.0

    @Mutes[0].EventHandler
    def on_first_mute(self, ctl):
        for inp in self.Inputs:
            inp.Value = 0.0
        total = len(self.Inputs)
        print(total)
"#;
    let lua = compile_to_lua(src);

    // 1. GetControls must emit `Count = props["number of inputs"].Value`
    assert!(
        lua.contains("Count = props[\"number of inputs\"].Value,"),
        "GetControls must emit property-driven Count:\n{lua}"
    );

    // 2. GetControlLayout must emit dynamic `for i = 1, props["number of inputs"].Value do`
    //    and branch on `== 1` ("inputs") vs `>= 2` ("inputs " .. tostring(i))
    assert!(
        lua.contains("for i = 1, props[\"number of inputs\"].Value do"),
        "GetControlLayout must loop 1..props['number of inputs'].Value:\n{lua}"
    );
    assert!(
        lua.contains("local ctl_key = (props[\"number of inputs\"].Value == 1) and \"inputs\" or (\"inputs \" .. tostring(i))"),
        "GetControlLayout must use 'inputs' when count == 1 and 'inputs i' when count >= 2:\n{lua}"
    );

    // 3. Explicitly indexed channels in UI layout: ch 1 branches on `== 1`, ch 2 guards `>= 2`
    assert!(
        lua.contains(
            "layout[(props[\"number of inputs\"].Value == 1) and \"mutes\" or \"mutes 1\"] = {"
        ),
        "Explicitly indexed channel 0 must branch on count == 1 vs >= 2:\n{lua}"
    );
    assert!(
        lua.contains(
            "if props[\"number of inputs\"].Value >= 2 then\n        layout[\"mutes 2\"] = {"
        ),
        "Explicitly indexed channel >= 1 must be guarded by count >= ch:\n{lua}"
    );

    // 4. Runtime @Inputs.EventHandler must respect `Properties["number of inputs"].Value == 1` vs `>= 2`
    assert!(
        lua.contains("if Properties[\"number of inputs\"].Value == 1 then\n        Controls[\"inputs\"].EventHandler = function(c)\n            DynamicInputPlugin_on_input(nil, c, 0)\n        end\n    else\n        for ix, ctl in ipairs(Controls[\"inputs\"]) do"),
        "Runtime @Inputs.EventHandler must handle count == 1 vs >= 2:\n{lua}"
    );

    // 5. Runtime indexing `self.Mutes[index]` must respect `== 1` vs `>= 2` via `__qsys_ctl`
    assert!(
        lua.contains("__qsys_ctl(Controls[\"mutes\"], (index) + 1, Properties[\"number of inputs\"].Value).Boolean"),
        "Runtime indexing on property-counted control must handle count == 1 vs >= 2:\n{lua}"
    );

    // 6. Runtime `for inp in self.Inputs:` and `len(self.Inputs)` must respect `== 1` vs `>= 2`
    assert!(
        lua.contains("for _, inp in ipairs((Properties[\"number of inputs\"].Value == 1) and { Controls[\"inputs\"] } or Controls[\"inputs\"]) do"),
        "Runtime iteration on property-counted control must handle count == 1 vs >= 2:\n{lua}"
    );
    assert!(
        lua.contains("local total = Properties[\"number of inputs\"].Value"),
        "len() on property-counted control must use Properties[...].Value:\n{lua}"
    );
}

#[test]
fn test_generic_property_typed_controls_and_data_accessor() {
    let src = r#"
class GenericControlPlugin(QSysPlugin):
    Type = Property.Enum(name="Type", choices=["Integer", "String", "Boolean"], default="String")

    Field = Control.[Type](name="field", min=0, max=100, default=0)
    SecondField = Control[Type](name="second", default="hello")
    ThirdField = Control.Generic(Type, name="third")

    layout = UI.Page(
        "Main",
        UI.GroupBox(
            "Dynamic Fields",
            UI.Field("Field", Field),
            UI.Field("Second", SecondField),
            UI.Field("Third", ThirdField),
        ),
    )

    @Field.EventHandler
    def on_field(self, ctl):
        current = ctl.Data
        print(current)
        print(ctl.String)
        print(ctl.Value)
        self.SecondField.Data = current
        self.ThirdField.Generic = self.SecondField.Any

    def runtime(self):
        self.Field.Data = "initial"
"#;
    let lua = compile_to_lua(src);

    // 1. GetControls(props) must branch on `props["Type"].Value` for each enum choice
    assert!(
        lua.contains("if props[\"Type\"].Value == \"Integer\" then")
            && lua.contains("ControlType = \"Knob\",")
            && lua.contains("ControlUnit = \"Integer\","),
        "GetControls must emit Integer Knob branch for 'Integer' choice:\n{lua}"
    );
    assert!(
        lua.contains("elseif props[\"Type\"].Value == \"String\" then")
            && lua.contains("ControlType = \"Text\",")
            && lua.contains("TextType = \"Text\","),
        "GetControls must emit Text branch for 'String' choice:\n{lua}"
    );
    assert!(
        lua.contains("elseif props[\"Type\"].Value == \"Boolean\" then")
            && lua.contains("ControlType = \"Button\",")
            && lua.contains("ButtonType = \"Toggle\","),
        "GetControls must emit Toggle Button branch for 'Boolean' choice:\n{lua}"
    );

    // 2. GetControlLayout(props) must emit dynamic Style expression driven by `props["Type"].Value`
    assert!(
        lua.contains("Style = (props[\"Type\"].Value == \"Integer\") and \"Knob\" or ((props[\"Type\"].Value == \"String\") and \"Text\" or (\"Button\")),"),
        "GetControlLayout must emit dynamic Style expression based on property choices:\n{lua}"
    );

    // 3. Runtime must emit `__qsys_get_data` and `__qsys_set_data` helpers and use them for `.Data`, `.Generic`, `.Any`
    assert!(
        lua.contains("local function __qsys_get_data(ctl, kind)")
            && lua.contains("local function __qsys_set_data(ctl, val, kind)"),
        "Runtime must emit __qsys_get_data and __qsys_set_data helpers:\n{lua}"
    );
    assert!(
        lua.contains("local current = __qsys_get_data(ctl, Properties[\"Type\"].Value)"),
        "Reading ctl.Data inside @Field.EventHandler must call __qsys_get_data with Properties['Type'].Value:\n{lua}"
    );
    assert!(
        lua.contains("__qsys_set_data(Controls[\"second\"], current, Properties[\"Type\"].Value)"),
        "Assigning self.SecondField.Data must call __qsys_set_data:\n{lua}"
    );
    assert!(
        lua.contains("__qsys_set_data(Controls[\"third\"], __qsys_get_data(Controls[\"second\"], Properties[\"Type\"].Value), Properties[\"Type\"].Value)"),
        "Reading .Any and writing .Generic must call __qsys_get_data / __qsys_set_data:\n{lua}"
    );
    // Standard fields (.String, .Value) still work directly
    assert!(
        lua.contains("print(ctl.String)") && lua.contains("print(ctl.Value)"),
        "Standard .String and .Value fields must still emit direct field access:\n{lua}"
    );
}

#[test]
fn test_compound_property_count_expressions_and_generic_controls() {
    let src = r#"
class WakeOnLan(QSysPlugin):
    Channels = Property.Integer(name="Channels", default=9, min=1, max=65535)
    Count = Property.Integer(name="Count", default=9, min=1, max=65535)
    MyType = Property.Enum(name="MyType", choices=["string"], default="string")
    Inputs = Control.Generic(MyType, name="Inputs", count=(Count*Channels))
    TrioInputs = Control.[MyType](name="TrioInputs", count=3*Channels)

    layout = UI.Page(
        "Main",
        UI.GroupBox(
            "Matrix",
            UI.Field("Input {i}", Inputs),
        ),
    )

    @Inputs.EventHandler
    def on_input(self, ctl, index):
        print(index, ctl.Data)
        self.TrioInputs[index*2].Value = ctl.Value
        self.TrioInputs[index*2+1].Value = ctl.Value
"#;
    let lua = compile_to_lua(src);

    assert!(
        lua.contains("Count = props[\"Count\"].Value * props[\"Channels\"].Value,"),
        "GetControls must emit compound Count = props['Count'].Value * props['Channels'].Value:\n{lua}"
    );
    assert!(
        lua.contains("Count = 3 * props[\"Channels\"].Value,"),
        "GetControls must emit scaled Count = 3 * props['Channels'].Value:\n{lua}"
    );
    assert!(
        lua.contains("for i = 1, props[\"Count\"].Value * props[\"Channels\"].Value do"),
        "GetControlLayout must loop 1..props['Count'].Value * props['Channels'].Value:\n{lua}"
    );
    assert!(
        lua.contains("if Properties[\"Count\"].Value * Properties[\"Channels\"].Value == 1 then"),
        "Runtime EventHandler must guard == 1 vs >= 2 with compound property expression:\n{lua}"
    );
    assert!(
        lua.contains("__qsys_ctl(Controls[\"TrioInputs\"], (index * 2) + 1, 3 * Properties[\"Channels\"].Value).Value = ctl.Value")
            && lua.contains("__qsys_ctl(Controls[\"TrioInputs\"], (index * 2) + 2, 3 * Properties[\"Channels\"].Value).Value = ctl.Value"),
        "Consecutive indexed assignments on property-counted controls must emit __qsys_ctl without leading '(':\n{lua}"
    );
}
