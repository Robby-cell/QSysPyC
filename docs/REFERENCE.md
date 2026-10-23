# QSysPy Complete Reference Guide

QSysPy compiles Python 3 syntax into standalone, dependency-free **Q-SYS Lua 5.3** scripts (`.lua`) and **Q-SYS Designer Plugins** (`.qplug`).

This document is the exhaustive reference for every target type, declaration class, `Control.*`, `Property.*`, `UI.*` layout component, multi-channel (`count > 1`) workflow, Q-SYS runtime object, Lua standard library module, and Python language feature supported by QSysPy.

---

## Table of Contents

1. [Compilation Targets](#1-compilation-targets)
2. [Plugin Metadata (`QSysPlugin` Class Attributes)](#2-plugin-metadata-qsysplugin-class-attributes)
3. [Controls (`Control.*`)](#3-controls-control)
   - [Common Control Options](#31-common-control-options)
   - [All `Control.*` Factories & Options](#32-all-control-factories--options)
   - [`ControlHandle` Runtime Properties & Indexing](#33-controlhandle-runtime-properties--indexing)
   - [Event Handlers (`@Control.EventHandler`)](#34-event-handlers-controleventhandler)
4. [Properties (`Property.*`)](#4-properties-property)
   - [All `Property.*` Factories & Options](#41-all-property-factories--options)
   - [`PropertyHandle` Runtime Access](#42-propertyhandle-runtime-access)
5. [Declarative UI Layout (`UI.*`)](#5-declarative-ui-layout-ui)
   - [How `GetControlLayout`, `GetPages`, and `PrettyName` (Pin Exposure) Work](#51-how-getcontrollayout-getpages-and-prettyname-pin-exposure-work)
   - [Four Supported Ways to Declare UI](#52-four-supported-ways-to-declare-ui)
   - [All `UI.*` Components & Options](#53-all-ui-components--options)
   - [Widget Styles & Colors](#54-widget-styles--colors)
6. [Multi-Channel Controls (`count > 1`)](#6-multi-channel-controls-count--1)
   - [Declaring Multi-Channel Controls](#61-declaring-multi-channel-controls)
   - [UI Layout Patterns for `count > 1`](#62-ui-layout-patterns-for-count--1)
   - [Runtime Indexing & Event Handlers for `count > 1`](#63-runtime-indexing--event-handlers-for-count--1)
7. [Design-Time vs. Runtime Lifecycle](#7-design-time-vs-runtime-lifecycle)
   - [Optional Design-Time Hook Overrides](#71-optional-design-time-hook-overrides)
8. [Q-SYS Runtime APIs (`qsys`)](#8-qsys-runtime-apis-qsys)
   - [`Timer` & `TimerHandle`](#81-timer--timerhandle)
   - [`UdpSocket`, `UdpSocketHandle`, & `UdpPacket`](#82-udpsocket-udpsockethandle--udppacket)
   - [`TcpSocket` & `TcpSocketHandle`](#83-tcpsocket--tcpsockethandle)
   - [`Controls` & `Properties` Global Tables](#84-controls--properties-global-tables)
   - [`qsys.lua()` Raw Lua Escape Hatch](#85-qsyslua-raw-lua-escape-hatch)
   - [Built-in Q-SYS Core Libraries](#86-built-in-qsys-core-libraries)
9. [Lua 5.3 Standard Library Modules](#9-lua-53-standard-library-modules)
10. [Python Built-ins & Language Features](#10-python-built-ins--language-features)
11. [End-to-End Examples](#11-end-to-end-examples)

---

## 1. Compilation Targets

QSysPy supports three compilation targets depending on your top-level structure:

| Target | How to Write It | Output Format | What Gets Generated |
| :--- | :--- | :--- | :--- |
| **Q-SYS Plugin** | Subclass `QSysPlugin` | `.qplug` | Complete Q-SYS plugin with `PluginInfo`, `GetProperties()`, `GetPages()`, `GetControls()`, `GetControlLayout()`, and runtime `if Controls then ... end` block. |
| **Text Controller / Script Component** | Subclass `TextController` | `.lua` | Standalone runtime Lua script wired to the component's existing `Controls` table (no design-time `PluginInfo` boilerplate). |
| **Procedural Script** | Top-level functions, classes, and statements (no `QSysPlugin` / `TextController`) | `.lua` | Direct Python-to-Lua 5.3 transpilation. |

### Imports for IDE Autocomplete (`.pyi` Stubs)

All Q-SYS symbols are globally available in the compiler, or can be explicitly imported from `qsys` for full IDE type checking (`pyright`, `mypy`, VS Code Pylance):

```python
from qsys import (
    QSysPlugin,
    TextController,
    Control,
    Property,
    UI,
    Controls,
    Properties,
    Timer,
    UdpSocket,
    TcpSocket,
    SerialPort,
    HttpClient,
    NamedControl,
    Notifications,
    Component,
    Log,
    System,
    Crypto,
    rapidjson,
    json,
    qsys,
)
```

---

## 2. Plugin Metadata (`QSysPlugin` Class Attributes)

When subclassing `QSysPlugin`, set class attributes at the top of the class body to populate the Q-SYS `PluginInfo` table:

```python
class MyAmplifier(QSysPlugin):
    name = "Acme~Pro Amplifier"
    version = "2.1.0"
    id = "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
    author = "Acme Audio"
    manufacturer = "Acme Corp"
    description = "8-channel network DSP amplifier plugin"
    show_debug = True
```

| Attribute | Type | Default | Q-SYS `PluginInfo` Field | Description |
| :--- | :--- | :--- | :--- | :--- |
| `name` | `str` | Class name | `Name` | Display name in Q-SYS Designer. Use `"Folder~Plugin Name"` to group inside subfolders in the Schematic Library. |
| `version` | `str` | `"1.0.0"` | `Version` | Semantic version string (`BuildVersion` is also auto-populated). |
| `id` | `str` | `""` | `Id` | Unique GUID string identifying the plugin across Q-SYS designs. |
| `author` | `str` | `"QSysPy"` | `Author` | Plugin author name. |
| `manufacturer` | `str` | *(omitted)* | `Manufacturer` | Hardware/software manufacturer name. |
| `description` | `str` | *(omitted)* | `Description` | Description shown in Q-SYS Designer's plugin properties panel. |
| `show_debug` | `bool` | `False` | `ShowDebug` | Enables the debug output console in Q-SYS Designer by default. |

---

## 3. Controls (`Control.*`)

Controls are declared as class attributes on a `QSysPlugin` or `TextController` class. In `QSysPlugin`, they automatically generate entries in `GetControls(props)` and bind to `Controls["<name>"]` at runtime.

### 3.1 Common Control Options

Every `Control.*` factory accepts the following arguments:

| Argument | Position / Keyword | Type | Default | Valid Values | Description |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `name` | 1st positional or `name=` | `str` | Python attribute name | Any string | The Q-SYS control name in `GetControls` and `Controls["<name>"]`. If omitted, defaults to the Python variable name (except `IPAddress`/`MACAddress` which default to `"IPAddress"`/`"MACAddress"`). |
| `pins` | `pins=` | `str` | `"Both"` | `"None"`, `"Input"`, `"Output"`, `"Both"` | Maps to Q-SYS `PinStyle`. Controls whether input/output wiring pins can be exposed on the schematic block. |
| `user_pin` | `user_pin=` | `bool` | `True` | `True`, `False` | Maps to Q-SYS `UserPin`. Allows users to toggle pin visibility in the Q-SYS Inspector. *(Note: Q-SYS also requires the control to appear in `GetControlLayout` with a `PrettyName`—which QSysPy's `UI.*` builder does automatically).* |
| `count` | `count=` | `int \| PropertyHandle` | `1` | Integer $\ge 1$, `Property.Integer` reference (`count=NumInputs`), or `props["..."].Value` | Number of control instances/channels (`Count` in Q-SYS). When bound to an integer `Property`, QSysPy automatically handles `Count == 1` vs `Count >= 2` across `GetControls`, `GetControlLayout`, and runtime `Controls`. |

---

### 3.2 All `Control.*` Factories & Options

#### `Control.Float` / `Control.Knob` / `Control.Fader`
Declares a Q-SYS `"Knob"` (`ControlType = "Knob"`) numeric control.
- `Control.Float` and `Control.Knob` default to a `"Knob"` widget in `UI.*`.
- `Control.Fader` is an alias that defaults to a `"Fader"` widget in `UI.*`.

```python
Gain = Control.Fader(min=-100.0, max=12.0, default=0.0, unit="dB", count=4)
Pan = Control.Knob("ChannelPan", min=-1.0, max=1.0, default=0.0, unit="Pan")
```

| Option | Type | Default | Valid Values |
| :--- | :--- | :--- | :--- |
| `name` | `str` | Attribute name | Any string (1st positional or `name=`) |
| `min` | `float \| int` | `0.0` | Any number (`Min` in Q-SYS) |
| `max` | `float \| int` | `1.0` | Any number (`Max` in Q-SYS) |
| `default` | `float \| int` | `min` | Any number (`DefaultValue` in Q-SYS) |
| `unit` | `str` | `"Float"` | `"Float"`, `"Integer"`, `"dB"`, `"Decibels"` (alias for `"dB"`), `"Hz"`, `"Hertz"` (alias for `"Hz"`), `"Percent"`, `"Seconds"`, `"Pan"` (`ControlUnit` in Q-SYS) |
| `pins` | `str` | `"Both"` | `"None"`, `"Input"`, `"Output"`, `"Both"` |
| `user_pin` | `bool` | `True` | `True`, `False` |
| `count` | `int` | `1` | Integer $\ge 1$ |

---

#### `Control.Button` / `Control.Boolean`
Declares a Q-SYS `"Button"` (`ControlType = "Button"`) control.
- `Control.Button` defaults to `type="Momentary"`.
- `Control.Boolean` defaults to `type="Toggle"`.

```python
Send = Control.Button("SendTrigger", type="Trigger")
Mute = Control.Boolean(type="Toggle", count=2)
```

| Option | Type | Default | Valid Values |
| :--- | :--- | :--- | :--- |
| `name` | `str` | Attribute name | Any string (1st positional or `name=`) |
| `type` | `str` | `"Momentary"` (`Button`) / `"Toggle"` (`Boolean`) | `"Momentary"`, `"Toggle"`, `"Trigger"` (`ButtonType` in Q-SYS) |
| `pins` | `str` | `"Both"` | `"None"`, `"Input"`, `"Output"`, `"Both"` |
| `user_pin` | `bool` | `True` | `True`, `False` |
| `count` | `int` | `1` | Integer $\ge 1$ |

---

#### `Control.Text`
Declares a Q-SYS `"Text"` (`ControlType = "Text"`) string box control.

```python
Hostname = Control.Text(default="192.168.1.100", type="Text")
Response = Control.Text(default="", type="ReadOnly", pins="Output")
```

| Option | Type | Default | Valid Values |
| :--- | :--- | :--- | :--- |
| `name` | `str` | Attribute name | Any string (1st positional or `name=`) |
| `default` | `str` | `""` | Any string (`DefaultValue` in Q-SYS) |
| `type` | `str` | `"Text"` | `"Text"`, `"ReadOnly"` |
| `pins` | `str` | `"Both"` | `"None"`, `"Input"`, `"Output"`, `"Both"` |
| `user_pin` | `bool` | `True` | `True`, `False` |
| `count` | `int` | `1` | Integer $\ge 1$ |

---

#### `Control.Indicator`
Declares a Q-SYS `"Indicator"` (`ControlType = "Indicator"`, `IndicatorType = "Led"`) status control.

```python
Online = Control.Indicator(pins="Output")
SignalLevel = Control.Indicator(unit="dB", count=8, pins="Output")
```

| Option | Type | Default | Valid Values |
| :--- | :--- | :--- | :--- |
| `name` | `str` | Attribute name | Any string (1st positional or `name=`) |
| `unit` | `str` | `"Float"` | `"Float"`, `"Integer"`, `"dB"`, `"Decibels"`, `"Hz"`, `"Hertz"`, `"Percent"`, `"Seconds"`, `"Pan"` |
| `pins` | `str` | `"Both"` | `"None"`, `"Input"`, `"Output"`, `"Both"` |
| `user_pin` | `bool` | `True` | `True`, `False` |
| `count` | `int` | `1` | Integer $\ge 1$ |

---

#### `Control.IPAddress` (or `Control.IpAddress`)
Declares a Q-SYS `"Text"` control configured for IPv4/IPv6 addresses.

```python
IPAddress = Control.IPAddress(default="192.168.1.50")
```

| Option | Type | Default | Valid Values |
| :--- | :--- | :--- | :--- |
| `name` | `str` | `"IPAddress"` | Any string (1st positional or `name=`) |
| `default` | `str` | `"255.255.255.255"` | Default IP string (`DefaultValue` in Q-SYS) |
| `pins` | `str` | `"Both"` | `"None"`, `"Input"`, `"Output"`, `"Both"` |
| `user_pin` | `bool` | `True` | `True`, `False` |
| `count` | `int` | `1` | Integer $\ge 1$ |

---

#### `Control.MACAddress` (or `Control.MacAddress`)
Declares a Q-SYS `"Text"` control configured for hardware MAC addresses.

```python
MACAddress = Control.MACAddress(default="00:11:22:33:44:55")
```

| Option | Type | Default | Valid Values |
| :--- | :--- | :--- | :--- |
| `name` | `str` | `"MACAddress"` | Any string (1st positional or `name=`) |
| `default` | `str` | `"00:00:00:00:00:00"` | Default MAC string (`DefaultValue` in Q-SYS) |
| `pins` | `str` | `"Both"` | `"None"`, `"Input"`, `"Output"`, `"Both"` |
| `user_pin` | `bool` | `True` | `True`, `False` |
| `count` | `int` | `1` | Integer $\ge 1$ |

---

#### `Control[Type]` / `Control.[Type]` / `Control.Generic(Type, ...)` (Property-Typed Generic Controls)
Declares a generic control whose Q-SYS `ControlType` (`"Text"`, `"Knob"`, `"Button"`, `"Indicator"`) and `GetControlLayout` `Style` dynamically depend on an enum or string `Property` (`props["<PropertyName>"].Value`):

```python
class DynamicTypePlugin(QSysPlugin):
    Type = Property.Enum(name="Type", choices=["Integer", "String", "Boolean"], default="String")

    # All three declaration forms are supported:
    Field = Control.[Type](name="field", min=0, max=100, default=0)
    SecondField = Control[Type](name="second", default="hello")
    ThirdField = Control.Generic(Type, name="third")
```

- **In `GetControls(props)`**: Emits `if props["Type"].Value == "Integer" then ... elseif props["Type"].Value == "String" then ...` with the matching Q-SYS `ControlType`, `ControlUnit`, `TextType`, or `ButtonType`.
- **In `GetControlLayout(props)`**: Automatically emits a property-driven `Style` expression (`"Knob"`, `"Text"`, `"Button"`, `"Led"`) unless overridden with an explicit `style=` argument.
- **At Runtime (`if Controls then`)**:
  - Standard fields (`.String`, `.Boolean`, `.Value`) continue to work directly as usual.
  - The generic accessor **`.Data`** (or aliases **`.Generic`** / **`.Any`**) automatically reads and writes whichever underlying field corresponds to the active property choice (`.String` for `"String"`/`"Text"`, `.Boolean` for `"Boolean"`/`"Toggle"`/`"Button"`, `.Value` for `"Integer"`/`"Float"`/`"Knob"`/`"Fader"`):
    ```python
    @Field.EventHandler
    def on_field(self, ctl):
        val = ctl.Data          # Reads .String, .Boolean, or .Value based on Properties["Type"].Value
        self.SecondField.Data = val
    ```

| Option | Type | Default | Valid Values |
| :--- | :--- | :--- | :--- |
| `type_prop` | `PropertyHandle \| str` | *(Required)* | Declared `Property.Enum` / `Property.String` identifier (or `props["..."].Value`) |
| `name` | `str` | Python attribute name | Any string |
| `min` | `float` | `0.0` | Minimum numeric value when numeric choice is active |
| `max` | `float` | `100.0` | Maximum numeric value when numeric choice is active |
| `default` | `float \| str` | `0.0` / `""` | Initial default numeric or string value |
| `pins` | `str` | `"Both"` | `"None"`, `"Input"`, `"Output"`, `"Both"` |
| `user_pin` | `bool` | `True` | `True`, `False` |
| `count` | `int \| PropertyHandle` | `1` | Integer $\ge 1$ or `Property.Integer` reference |

---

### 3.3 `ControlHandle` Runtime Properties & Indexing

At runtime (inside `__init__`, event handlers, and helper methods), accessing `self.<ControlName>` returns a `ControlHandle` (or array of `ControlHandle`s when `count > 1`).

| Property | Type | Description |
| :--- | :--- | :--- |
| `.Data` (or `.Generic`, `.Any`) | `float \| int \| str \| bool` | **Generic property-typed value accessor.** Reads or writes `.String`, `.Boolean`, or `.Value` dynamically based on the control's active property type (or statically for fixed-type controls). |
| `.Value` | `float` | Numeric floating-point value of the control (`0.0` or `1.0` for buttons/booleans). |
| `.String` | `str` | Formatted text representation of the control value (e.g., `"-6.0 dB"` or text input string). |
| `.Boolean` | `bool` | Boolean state of the control (`True` when `.Value >= 0.5`). |
| `.Values` | `list[float]` | Multi-value list for vector controls. |
| `.Strings` | `list[str]` | List of string choices for combo-box controls (`Choices` in Q-SYS). |
| `.IsDisabled` | `bool` | Disables/greys out the control in the UI when `True`. |
| `.IsInvisible` | `bool` | Hides the control in the UI when `True`. |
| `.Color` | `str` | Runtime CSS/hex color override (e.g. `"#22c55e"` or `"Red"`). |
| `.Legend` | `str` | Dynamic button/control text legend. |
| `.RampTime` | `float` | Time in seconds to ramp to the next `.Value` assignment. |
| `.EventHandler` | `Callable[[ControlHandle], None]` | Callback invoked when the control changes value or is triggered. |

---

### 3.4 Event Handlers (`@Control.EventHandler`)

You can bind control event handlers declaratively using decorators on methods:

```python
class MyPlugin(QSysPlugin):
    Send = Control.Button(type="Trigger")
    Gain = Control.Fader(min=-100.0, max=12.0, default=0.0, unit="dB", count=4)

    # 1. Single control handler
    @Send.EventHandler
    def on_send(self, ctl):
        print("Send triggered!", ctl.Value)

    # 2. Multi-channel (count > 1) handler across ALL channels
    #    `index` is 0-based in Python (0, 1, 2, 3)!
    @Gain.EventHandler
    def on_any_gain(self, ctl, index):
        print(f"Channel {index + 1} changed to {ctl.String}")

    # 3. Specific channel handler (0-based index)
    @Gain[0].EventHandler
    def on_ch1_gain(self, ctl):
        print("Channel 1 specifically changed:", ctl.Value)
```

---

## 4. Properties (`Property.*`)

Properties appear in Q-SYS Designer's **Properties Inspector** panel when the plugin block is selected on the schematic. They are configured at design time and read-only at runtime.

### 4.1 All `Property.*` Factories & Options

#### `Property.Integer`
```python
Port = Property.Integer("Port", default=9, min=1, max=65535, description="Target UDP port")
```
| Option | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `name` | `str` | Attribute name | Property name in `GetProperties()` and `Properties["<name>"]` (1st positional or `name=`). |
| `default` | `int` | `0` | Default integer value (`Value` in Q-SYS). |
| `min` | `int` | `0` (or `default` if negative) | Minimum allowed integer (`Min` in Q-SYS). |
| `max` | `int` | `100` (or `default` if $>100$) | Maximum allowed integer (`Max` in Q-SYS). |
| `description` | `str` | *(omitted)* | Tooltip/help text (`Description` in Q-SYS). |

---

#### `Property.Float` / `Property.Double`
```python
PollInterval = Property.Float(default=1.5, min=0.1, max=60.0, description="Poll interval (s)")
```
| Option | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `name` | `str` | Attribute name | Property name (1st positional or `name=`). |
| `default` | `float \| int` | `0.0` | Default floating-point value (`Value` in Q-SYS, `Type = "double"`). |
| `min` | `float \| int` | `0.0` | Minimum allowed value (`Min` in Q-SYS). |
| `max` | `float \| int` | `100.0` | Maximum allowed value (`Max` in Q-SYS). |
| `description` | `str` | *(omitted)* | Tooltip/help text. |

---

#### `Property.String`
```python
DefaultHost = Property.String(default="192.168.1.10", description="Default IP or hostname")
```
| Option | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `name` | `str` | Attribute name | Property name (1st positional or `name=`). |
| `default` | `str` | `""` | Default string value (`Value` in Q-SYS, `Type = "string"`). |
| `description` | `str` | *(omitted)* | Tooltip/help text. |

---

#### `Property.Boolean`
```python
AutoReconnect = Property.Boolean(default=True, description="Reconnect automatically on disconnect")
```
| Option | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `name` | `str` | Attribute name | Property name (1st positional or `name=`). |
| `default` | `bool` | `False` | Default boolean value (`Value` in Q-SYS, `Type = "boolean"`). |
| `description` | `str` | *(omitted)* | Tooltip/help text. |

---

#### `Property.Enum`
```python
Protocol = Property.Enum(choices=["UDP", "TCP", "HTTP"], default="UDP", description="Transport protocol")
```
| Option | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `choices` | `list[str]` | **Required** | Non-empty list of string choices (`Choices` in Q-SYS, `Type = "enum"`). Can be passed as 1st positional arg or `choices=[...]`. |
| `default` | `str` | `choices[0]` | Initial selected choice (`Value` in Q-SYS). Must be an element of `choices`. |
| `name` | `str` | Attribute name | Property name (1st positional if `choices=` is keyword, or `name=`). |
| `description` | `str` | *(omitted)* | Tooltip/help text. |

---

### 4.2 `PropertyHandle` Runtime Access

At runtime, accessing `self.<PropertyName>` or `self.<PropertyName>.Value` compiles to `Properties["<PropertyName>"].Value`:

```python
def __init__(self):
    port = self.Port          # -> Properties["Port"].Value
    proto = self.Protocol.Value  # -> Properties["Protocol"].Value
```

---

## 5. Declarative UI Layout (`UI.*`)

In native Q-SYS Lua, building a plugin interface requires manually calculating pixel `(x, y)` coordinates and `(w, h)` bounding boxes for every single control and graphic in `GetControlLayout(props)`.

QSysPy replaces manual pixel math with a declarative, Flexbox-like **`UI.*` layout engine** that compiles directly to Q-SYS `layout` and `graphics` tables at compile time.

### 5.1 How `GetControlLayout`, `GetPages`, and `PrettyName` (Pin Exposure) Work

1. **Zero Runtime Overhead**: All `UI.*` calls are solved at **compile time** into exact `(Position, Size)` coordinates and emitted as standard Lua `layout["..."] = { PrettyName = "...", Style = "...", Position = { x, y }, Size = { w, h } }` and `graphics` entries inside `GetControlLayout(props)` and `GetPages(props)`.
2. **Automatic `PrettyName` for Pin Exposure**:
   > **Important Q-SYS Requirement**: In Q-SYS Designer, setting `UserPin = true` and `PinStyle = "Both"` in `GetControls` is **not enough** to expose a control's pin! Q-SYS Designer only lists a control in the Inspector's Pin panel if that control is also placed in `GetControlLayout` **with a `PrettyName`**.
   
   QSysPy automatically generates and emits a hierarchical `PrettyName` on **every** control placed via `UI.Field`, `UI.Control`, or bare control reference:
   - If the control is inside a `UI.GroupBox("Network", ...)`, QSysPy prefixes the group title with `~` (e.g. `PrettyName = "Network~IP Address"`), which Q-SYS Designer uses to organize pins into collapsible folders!
   - You can always override `pretty_name="..."` or `pretty_names=[...]` explicitly.

---

### 5.2 Four Supported Ways to Declare UI

You can declare your plugin's UI using whichever style fits your plugin best:

#### Style A: Single-Page Class Attribute (`layout = UI.Page(...)`)
```python
class MyPlugin(QSysPlugin):
    IPAddress = Control.IPAddress()
    Send = Control.Button()

    layout = UI.Page(
        "Main",
        UI.GroupBox(
            "Connection",
            UI.Field("Target IP", IPAddress),
            UI.Field("Trigger", Send),
        ),
    )
```

#### Style B: Multi-Page Class Attribute (`pages = [...]` or `layout = UI.Pages(...)`)
Generates a multi-page tabbed plugin (`GetPages(props)` + `CurrentPage` branching in `GetControlLayout(props)`):
```python
class MyPlugin(QSysPlugin):
    Gain = Control.Fader(min=-100.0, max=12.0, default=0.0, unit="dB", count=4)
    IPAddress = Control.IPAddress()

    pages = [
        UI.Page(
            "Mixer",
            UI.GroupBox("Channels", UI.Field("Ch {i}", Gain, direction="row")),
        ),
        UI.Page(
            "Setup",
            UI.GroupBox("Network", UI.Field("Device IP", IPAddress)),
        ),
    ]
```

#### Style C: Inside `def get_control_layout(self, props):`
If you prefer writing a method (or migrating from an existing plugin), returning a `UI.Page(...)` or `UI.Pages(...)` from `get_control_layout` is automatically lowered by the compiler into Q-SYS `GetPages` and `GetControlLayout`:
```python
    def get_control_layout(self, props):
        return UI.Page(
            "Main",
            UI.Header("Plugin Title"),
            UI.GroupBox(
                "Controls",
                UI.Field("Device IP", self.IPAddress),
            ),
        )
```
*(Even if you return `return layout, graphics` where `layout = UI.Page(...)`, QSysPy intercepts it and compiles the `UI.Page` into valid Q-SYS `layout` and `graphics` tables!)*

#### Style D: Raw Imperative Lua Tables in `def get_control_layout(self, props):`
For full manual control or dynamic `props`-dependent sizing, you can populate `layout = {}` and `graphics = []` dicts/lists directly and `return layout, graphics`.

---

### 5.3 All `UI.*` Components & Options

#### `UI.Page(title="Main", *children, **kwargs)`
Defines a top-level plugin page tab.

| Argument | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `title` / `name` | `str` | `"Main"` | 1st positional arg or `title=` / `name=`. Page tab name in Q-SYS `GetPages(props)`. |
| `*children` | `UINode \| ControlHandle` | `()` | Variadic child nodes or `children=[...]` list (supports `*` unpacking of list comprehensions). |
| `padding` | `float \| int` | `16.0` | Outer page padding in pixels. |
| `gap` | `float \| int` | `12.0` | Vertical spacing between direct child nodes in pixels. |
| `width` | `float \| int \| None` | `None` (auto-inferred, min `280.0`) | Optional explicit page content width in pixels. |

---

#### `UI.Pages(*pages, **kwargs)`
Wraps multiple `UI.Page(...)` definitions into a tabbed multi-page layout.
- Accepts `UI.Page(...)` children positionally (`UI.Pages(page1, page2)`) or via `pages=[...]` / `children=[...]`.

---

#### `UI.GroupBox(title="", *children, **kwargs)`
*(Aliases: `UI.Section`, `UI.Card`, `UI.Panel`)*  
Draws a Q-SYS `GroupBox` container graphic around its children and automatically scopes child control `PrettyName`s as `"<title>~<control>"`.

| Argument | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `title` / `text` | `str` | `""` | 1st positional string or `title=` / `text=`. Header text shown on the GroupBox border. |
| `*children` | `UINode \| ControlHandle` | `()` | Variadic child nodes or `children=[...]`. |
| `direction` | `str` | `"col"` | Layout flow of children: `"col"` (`"column"`, `"vertical"`) or `"row"` (`"horizontal"`). |
| `gap` | `float \| int` | `10.0` | Spacing in pixels between children. |
| `padding` | `float \| int` | `12.0` | Inner padding in pixels (extra top clearance is automatically added when `title` is non-empty). |
| `width` | `float \| int \| None` | `None` (auto) | Optional explicit width in pixels. |
| `height` | `float \| int \| None` | `None` (auto) | Optional explicit height in pixels. |
| `color` | `str \| tuple[int,int,int]` | `(30, 30, 30)` | Header/title text color (`Color` in Q-SYS). |
| `fill` | `str \| tuple[int,int,int]` | `(242, 242, 242)` | Background fill color (`Fill` in Q-SYS). |
| `radius` | `int` | `8` | Corner radius in pixels (`CornerRadius` in Q-SYS). |
| `stroke_width` / `stroke` | `int` | `1` | Border stroke thickness in pixels (`StrokeWidth` in Q-SYS). |

---

#### `UI.Row(*children, **kwargs)`
*(Alias: `UI.HStack`)*  
Arranges child nodes horizontally from left to right.

| Argument | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `*children` | `UINode \| ControlHandle` | `()` | Variadic child nodes or `children=[...]`. |
| `gap` | `float \| int` | `8.0` | Horizontal spacing in pixels between children. |
| `padding` | `float \| int` | `0.0` | Inner padding around the row in pixels. |

---

#### `UI.Col(*children, **kwargs)`
*(Aliases: `UI.Column`, `UI.VStack`, `UI.Div`)*  
Arranges child nodes vertically from top to bottom.

| Argument | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `*children` | `UINode \| ControlHandle` | `()` | Variadic child nodes or `children=[...]`. |
| `gap` | `float \| int` | `8.0` | Vertical spacing in pixels between children. |
| `padding` | `float \| int` | `0.0` | Inner padding around the column in pixels. |

---

#### `UI.Field(label, control, **kwargs)`
Pairs a Q-SYS `Label` text graphic with a declared `ControlHandle` (`self.Gain`, `Gain`, or `self.Gain[0]`), automatically configures `PrettyName`, and auto-expands multi-channel (`count > 1`) controls.

```python
UI.Field("Master Gain", self.Gain, style="Fader", label_pos="top", color="#3b82f6")
```

| Argument | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `label` | `str` | **Required** | 1st positional or `label=`. Text for the `Label` graphic and default `PrettyName`. Supports `{i}` (1-based channel number) and `{ch}` when `count > 1`. |
| `control` | `ControlHandle` | **Required** | 2nd positional or `control=`. Reference to a declared control (`self.Gain`, `Gain`, or `self.Gain[0]`). |
| `style` | `str` | Auto-inferred from `Control.*` | Widget style override (`"Fader"`, `"Knob"`, `"Button"`, `"Text"`, `"Led"`, `"Meter"`, `"ComboBox"`, `"None"`). |
| `label_pos` | `str` | `"top"` | Where to place the label relative to the control: `"top"` (stacked above) or `"left"` (`"inline"`, `"side"`, `"horizontal"` — side-by-side). |
| `label_width` | `float \| int` | `88.0` | Width of the label in pixels when `label_pos="left"`. |
| `pretty_name` | `str` | `label` (or `"<GroupBox>~<label>"`) | Explicit Q-SYS `PrettyName` override for pin exposure. Supports `{i}` / `{ch}` for `count > 1`. |
| `pretty_names` | `list[str]` | `None` | Per-channel `PrettyName` list when expanding a `count > 1` control (e.g. `["Left", "Right"]`). |
| `labels` | `list[str]` | `None` | Per-channel UI label list when expanding a `count > 1` control (e.g. `["Ch 1 (L)", "Ch 2 (R)"]`). |
| `direction` | `str` | `"row"` for Knobs/Faders/Indicators, `"col"` for Text/Buttons | Direction (`"row"` or `"col"`) when auto-expanding an unindexed `count > 1` control. |
| `gap` | `float \| int` | `8.0` | Spacing in pixels between expanded channels when `count > 1`. |
| `index` | `int` | `None` | Optional 0-based channel index (equivalent to passing `self.Gain[index]`). |
| `legend` | `str` | `None` | Static button legend (`Legend` in Q-SYS). |
| `width` | `float \| int` | Default for style | Explicit control widget width in pixels. |
| `height` | `float \| int` | Default for style | Explicit control widget height in pixels. |
| `size` | `tuple[float, float]` | Default for style | Explicit `(width, height)` tuple in pixels. |
| `color` | `str \| tuple[int,int,int]` | `None` | Control widget color (`Color` in Q-SYS). |
| `margin` | `int` | `None` | Q-SYS widget `Margin` property. |
| `read_only` | `bool` | `True` for `Control.Text(type="ReadOnly")`, else `False` | Sets `IsReadOnly = true` on the layout entry. |
| `invisible` | `bool` | `False` | Sets `IsInvisible = true` on the layout entry. |

---

#### `UI.Control(control, **kwargs)`
Places a declared `ControlHandle` (`self.Gain`, `Gain`, or `self.Gain[0]`) without a separate label graphic (unless `label=` is explicitly passed). Still emits `PrettyName` automatically so pins can be exposed!

```python
UI.Control(self.Send, style="Button", legend="WAKE", width=96, height=32, color="Green")
```
- Accepts all the same keyword arguments as `UI.Field` (`style`, `pretty_name`, `pretty_names`, `label`, `labels`, `label_pos`, `label_width`, `direction`, `gap`, `index`, `legend`, `width`, `height`, `size`, `color`, `margin`, `read_only`, `invisible`).
- **Bare Control Shorthand**: Passing `self.Gain` or `self.Gain[0]` directly inside `UI.Page`, `UI.GroupBox`, `UI.Row`, or `UI.Col` is shorthand for `UI.Control(self.Gain)`.

---

#### `UI.Header(text="", **kwargs)`
*(Aliases: `UI.H1`, `UI.H2`)*  
Emits a Q-SYS `Header` banner graphic (`Type = "Header"`).

| Argument | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `text` / `title` | `str` | `""` | 1st positional or `text=` / `title=`. Banner text. |
| `width` | `float \| int \| None` | `None` (fills container width, min `200.0`) | Explicit width in pixels. |
| `height` | `float \| int` | `28.0` | Banner height in pixels. |
| `font_size` | `int` | `14` | Text size (`FontSize` in Q-SYS). |
| `color` | `str \| tuple[int,int,int]` | `(51, 51, 51)` | Header color (`Color` in Q-SYS). |

---

#### `UI.Label(text="", **kwargs)`
*(Alias: `UI.Text`)*  
Emits a standalone Q-SYS `Label` text graphic (`Type = "Label"`).

| Argument | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `text` | `str` | `""` | 1st positional or `text=`. Label text. |
| `width` | `float \| int \| None` | `None` (auto-estimated from text length, min `80.0`) | Explicit width in pixels. |
| `height` | `float \| int` | `22.0` | Label height in pixels. |
| `font_size` | `int` | `12` | Text size (`FontSize` in Q-SYS). |
| `align` | `str` | `"Left"` | Horizontal alignment (`HTextAlign`): `"Left"`, `"Center"`, or `"Right"`. |
| `color` | `str \| tuple[int,int,int]` | `(40, 40, 40)` | Text color (`Color` in Q-SYS). |

---

#### `UI.Spacer(width=12.0, height=12.0, **kwargs)`
Inserts empty whitespace inside a `UI.Row` or `UI.Col`.
- Accepts `UI.Spacer(16)` (sets both width and height to `16`), `UI.Spacer(24, 8)`, or keyword arguments `width=`, `height=`, `size=`.

---

### 5.4 Widget Styles & Colors

#### Supported `style=` Values & Default Widget Sizes

| Control Declaration | Default `style` | Default `(width, height)` | Accepted `style=` Overrides & Aliases |
| :--- | :--- | :--- | :--- |
| `Control.Float` / `Control.Knob` | `"Knob"` | `(56, 56)` | `"Knob"` (`"dial"`, `"rotary"`), `"Fader"` (`"slider"`), `"Text"`, `"Meter"`, `"None"` |
| `Control.Fader` | `"Fader"` | `(48, 128)` | `"Fader"` (`"slider"`), `"Knob"`, `"Text"`, `"Meter"`, `"None"` |
| `Control.Button` / `Control.Boolean` | `"Button"` | `(84, 28)` | `"Button"` (`"btn"`, `"toggle"`, `"trigger"`), `"Led"`, `"None"` |
| `Control.Text` / `IPAddress` / `MACAddress` | `"Text"` | `(160, 24)` | `"Text"` (`"textbox"`, `"input"`), `"ComboBox"`, `"None"` |
| `Control.Indicator` | `"Led"` | `(24, 24)` | `"Led"` (`"indicator"`), `"Meter"` (`"level"` — default `(28, 110)`), `"Text"`, `"None"` |

#### Supported Color Formats (`color=` and `fill=`)
Colors on `UI.GroupBox`, `UI.Field`, `UI.Control`, `UI.Header`, and `UI.Label` can be passed in three ways:
1. **Hex string**: `"#3b82f6"` or `"#f00"`
2. **RGB 3-tuple**: `(59, 130, 246)`
3. **Named color preset** (case-insensitive):
   - `"Red"` `(239, 68, 68)`
   - `"Green"` `(34, 197, 94)`
   - `"Blue"` `(59, 130, 246)`
   - `"Yellow"` / `"Amber"` `(234, 179, 8)`
   - `"Orange"` `(249, 115, 22)`
   - `"Purple"` `(168, 85, 247)`
   - `"Cyan"` / `"Teal"` `(6, 182, 212)`
   - `"White"` `(255, 255, 255)`
   - `"Black"` `(0, 0, 0)`
   - `"Gray"` / `"Grey"` `(156, 163, 175)`
   - `"Dark"` `(31, 41, 55)`

---

## 6. Multi-Channel Controls (`count > 1`)

Q-SYS uses `Count = N` in `GetControls` for multi-channel controls (e.g. an 8-channel mixer gain, mute buttons, or signal meters). In raw Q-SYS Lua, `Controls["Gain"]` becomes a 1-based table `Controls["Gain"][1] ... Controls["Gain"][N]` and `GetControlLayout` keys must be space-separated `"Gain 1" ... "Gain N"`.

QSysPy makes multi-channel controls seamless across declarations, UI layouts, and runtime code:

### 6.1 Declaring Multi-Channel Controls (Static or Property-Driven)

You can pass either a static integer (`count=4`) **or** an integer `Property` (`count=NumInputs` or `count=props["number of inputs"].Value`):

```python
class MixerPlugin(QSysPlugin):
    # Dynamic channel count configured in Q-SYS Designer Inspector
    NumInputs = Property.Integer("number of inputs", default=4, min=1, max=16)

    # Bound to the Property (or static integer `count=4`)
    Gain = Control.Fader(min=-100.0, max=12.0, default=0.0, unit="dB", count=NumInputs)
    Mute = Control.Boolean(type="Toggle", count=props["number of inputs"].Value)
    Signal = Control.Indicator(count=NumInputs, pins="Output")
```

> **Automatic `1` vs `2+` Handling**: In Q-SYS Lua, when a property-driven `Count` is set to `1`, Q-SYS names the layout key `"Gain"` (without `" 1"`) and makes `Controls["Gain"]` a single `ControlHandle` object instead of a 1-element table. When `Count >= 2`, Q-SYS uses `"Gain 1" .. "Gain N"` and makes `Controls["Gain"]` a 1-based array table.
>
> When you use `count=NumInputs` in QSysPy, the compiler **automatically emits `== 1` vs `>= 2` guards** in:
> - `GetControls(props)` (`Count = props["number of inputs"].Value`)
> - `GetControlLayout(props)` (`for i = 1, props["number of inputs"].Value do` with `"Gain"` when `1` and `"Gain " .. tostring(i)` when `>= 2`)
> - Runtime `@Gain.EventHandler`, `@Gain[0].EventHandler`, `self.Gain[i]`, `for ctl in self.Gain:`, and `len(self.Gain)`!

### 6.2 UI Layout Patterns for `count > 1`

You have **three ergonomic ways** to lay out multi-channel controls:

#### Pattern 1: Automatic Expansion with `{i}` Template or `labels=[...]`
Passing an unindexed `count > 1` control to `UI.Field` or `UI.Control` automatically expands all channels!
- Use `{i}` (or `{ch}`) in `label` or `pretty_name` to interpolate the 1-based channel number (`1, 2, 3, ...`).
- Or pass `labels=[...]` and `pretty_names=[...]` for custom per-channel names (e.g. Stereo Left/Right).
- Control the expansion axis with `direction="row"` or `direction="col"`.

```python
# Expands into 4 side-by-side faders labeled "Ch 1", "Ch 2", "Ch 3", "Ch 4"
# with PrettyNames "Faders~Ch 1", "Faders~Ch 2", etc. and keys "Gain 1" .. "Gain 4"
UI.GroupBox(
    "Faders",
    UI.Field("Ch {i}", Gain, direction="row"),
    UI.Field("Mute {i}", Mute, direction="row", color="Red"),
)

# Custom stereo labels & PrettyNames:
StereoGain = Control.Knob(min=-100.0, max=12.0, default=0.0, unit="dB", count=2)
UI.Field(
    "Gain",
    StereoGain,
    labels=["Left", "Right"],
    pretty_names=["Output~Left Gain", "Output~Right Gain"],
    direction="row",
)
```

#### Pattern 2: Individual Channel Placement via 0-Based Indexing (`Gain[0]`, `Gain[1]`)
Index a multi-channel control with 0-based Python indices (`0 .. count - 1`) to place individual channels anywhere in your layout:
- `Gain[0]` maps to Q-SYS layout key `"Gain 1"` and runtime `Controls["Gain"][1]`.

```python
UI.Row(
    UI.Field("Main L", Gain[0]),
    UI.Field("Main R", Gain[1]),
    UI.Spacer(16),
    UI.Field("Sub 1", Gain[2]),
    UI.Field("Sub 2", Gain[3]),
)
```

#### Pattern 3: Channel Strips via List Comprehensions (`*[... for i in range(N)]`)
When each channel strip combines multiple controls (e.g. Signal LED + Gain Fader + Mute Button per channel), use a Python list comprehension with `*` unpacking:

```python
UI.GroupBox(
    "Channel Strips",
    UI.Row(
        *[
            UI.Col(
                UI.Field(f"Sig {i + 1}", Signal[i]),
                UI.Field(f"Ch {i + 1}", Gain[i], style="Fader"),
                UI.Field(f"Mute {i + 1}", Mute[i], color="Red"),
            )
            for i in range(4)
        ],
        gap=14,
    ),
)
```
*(Compile-time constant expressions like `i + 1` inside f-strings are evaluated automatically at compile time!)*

---

### 6.3 Runtime Indexing & Event Handlers for `count > 1`

1. **0-Based Indexing in Python $\rightarrow$ 1-Based in Lua**:
   ```python
   self.Gain[0].Value = -6.0       # Compiles to: Controls["Gain"][1].Value = -6.0
   self.Mute[i].Boolean = True     # Compiles to: Controls["Mute"][i + 1].Boolean = true
   ```
2. **Iterating Over All Channels**:
   ```python
   for ctl in self.Gain:
       ctl.Value = 0.0             # Compiles to: for _, ctl in ipairs(Controls["Gain"]) do ...
   ```
3. **All-Channel Event Handler (`@Gain.EventHandler`)**:
   If your handler method takes `(self, ctl, index)`, `index` is automatically passed as a **0-based** Python index (`0 .. count - 1`), so `self.Mute[index]` lines up without off-by-one bugs:
   ```python
   @Gain.EventHandler
   def on_gain_change(self, ctl, index):
       # `index` is 0-based (0, 1, 2, 3)
       print(f"Channel {index + 1} gain is now {ctl.Value} dB")
       if ctl.Value <= -100.0:
           self.Mute[index].Boolean = True
   ```
4. **Single-Channel Event Handler (`@Gain[0].EventHandler`)**:
   ```python
   @Gain[0].EventHandler
   def on_ch1_gain(self, ctl):
       print("Channel 1 gain changed:", ctl.Value)
   ```

---

## 7. Design-Time vs. Runtime Lifecycle

A compiled Q-SYS `.qplug` file runs in two phases inside Q-SYS Designer:
1. **Design-Time Phase**: Q-SYS Designer calls `GetProperties()`, `GetPages(props)`, `GetControls(props)`, and `GetControlLayout(props)` to build the schematic block, pins, and control panel. **`Controls` and `Properties` do not exist yet.**
2. **Runtime Phase (`if Controls then ... end`)**: When the design is emulated or deployed to a Q-SYS Core, QSysPy instantiates your plugin class (`__init__`) and attaches all `@Control.EventHandler` callbacks.

> **Compile-Time Safety**: QSysPy tracks execution contexts (`DesignTime`, `Runtime`, `Both`) and emits a compile-time error if you accidentally reference `Controls` or `Properties` inside a design-time method, or `UI.*` inside runtime methods.

### 7.1 Optional Design-Time Hook Overrides

In addition to declarative `Control.*`, `Property.*`, and `UI.*` attributes, you can optionally define these methods on your `QSysPlugin` class to dynamically inspect `props` (the design-time properties table) and customize the plugin at design time:

| Method Signature | Compiles To | Purpose |
| :--- | :--- | :--- |
| `def get_properties(self):` | `GetProperties()` | Append or return custom property dicts. |
| `def get_pages(self, props):` | `GetPages(props)` | Dynamically vary pages based on `props`. |
| `def get_controls(self, props):` | `GetControls(props)` | Dynamically add controls based on `props` (e.g. channel count property). |
| `def get_control_layout(self, props):` | `GetControlLayout(props)` | Return a `UI.Page(...)` / `UI.Pages(...)` or `(layout, graphics)` tuple. |
| `def get_color(self, props):` | `GetColor(props)` | Return an RGB list `[r, g, b]` for the schematic block color in Q-SYS Designer. |
| `def get_pretty_name(self, props):` | `GetPrettyName(props)` | Return a dynamic string label shown on the schematic block in Q-SYS Designer. |
| `def get_pins(self, props):` | `GetPins(props)` | Return custom audio/serial/network hardware pin definitions. |
| `def get_components(self, props):` | `GetComponents(props)` | Embed internal Q-SYS DSP components inside the plugin. |
| `def get_wiring(self, props):` | `GetWiring(props)` | Wire embedded components to plugin pins. |
| `def rectify_properties(self, props):` | `RectifyProperties(props)` | Hide or adjust design-time properties dynamically based on other property values. |

---

## 8. Q-SYS Runtime APIs (`qsys`)

QSysPy automatically handles Lua's `.` vs `:` calling convention—in Python you always write `.` (`self.timer.Start(1.0)`), and QSysPy emits `self.timer:Start(1.0)` for handle methods and `Timer.New()` for static factory calls.

### 8.1 `Timer` & `TimerHandle`

| Symbol | Signature | Description |
| :--- | :--- | :--- |
| `Timer.New()` | `() -> TimerHandle` | Creates a new Q-SYS event-loop timer. |
| `Timer.CallAfter(callback, delay)` | `(Callable[[], None], float) -> None` | Invokes `callback` once after `delay` seconds. |
| `timer.Start(seconds)` | `(float) -> None` | Starts or restarts the repeating timer with interval `seconds` (emits `:Start`). |
| `timer.Stop()` | `() -> None` | Stops the timer (emits `:Stop`). |
| `timer.Reset()` | `() -> None` | Resets the timer counter (emits `:Reset`). |
| `timer.EventHandler` | `Callable[[TimerHandle], None]` | Callback invoked every time the timer interval elapses. |

```python
self.poll_timer = Timer.New()
self.poll_timer.EventHandler = self.on_tick
self.poll_timer.Start(1.0)
```

---

### 8.2 `UdpSocket`, `UdpSocketHandle`, & `UdpPacket`

| Symbol | Signature | Description |
| :--- | :--- | :--- |
| `UdpSocket.New()` | `() -> UdpSocketHandle` | Creates a new UDP socket instance. |
| `udp.Open(ip=None, port=None)` | `(str?, int?) -> None` | Binds and opens the UDP socket on optional local `ip` and `port` (emits `:Open`). |
| `udp.Close()` | `() -> None` | Closes the UDP socket (emits `:Close`). |
| `udp.Send(ip, port, data)` | `(str, int, str) -> None` | Sends payload `data` to `ip:port` (emits `:Send`). |
| `udp.JoinMulticast(address)` | `(str) -> None` | Joins multicast group `address` (emits `:JoinMulticast`). |
| `udp.LeaveMulticast(address)` | `(str) -> None` | Leaves multicast group `address` (emits `:LeaveMulticast`). |
| `udp.Data` | `Callable[[UdpSocketHandle, UdpPacket], None]` | Callback invoked when a UDP datagram arrives. |
| `udp.Closed` | `Callable[[UdpSocketHandle], None]` | Callback invoked when the socket closes. |
| `packet.Address` | `str` | Sender IPv4/IPv6 address on incoming `UdpPacket`. |
| `packet.Port` | `int` | Sender UDP port on incoming `UdpPacket`. |
| `packet.Data` | `str` | Raw payload string on incoming `UdpPacket`. |

---

### 8.3 `TcpSocket` & `TcpSocketHandle`

| Symbol | Signature | Description |
| :--- | :--- | :--- |
| `TcpSocket.New()` | `() -> TcpSocketHandle` | Creates a new TCP client socket. |
| `tcp.Connect(ip, port)` | `(str, int) -> None` | Connects to remote `ip` and `port` (emits `:Connect`). |
| `tcp.Disconnect()` | `() -> None` | Closes the TCP connection (emits `:Disconnect`). |
| `tcp.Write(data)` | `(str) -> None` | Transmits string/binary `data` over the socket (emits `:Write`). |
| `tcp.Read(length=None)` | `(int?) -> str` | Reads up to `length` bytes from the receive buffer (emits `:Read`). |
| `tcp.ReadLine(eol=None)` | `(Any?) -> str \| None` | Reads a line using e.g. `TcpSocket.EOL.Any`, or `None` if no full line is buffered (emits `:ReadLine`). |
| `tcp.IsConnected` | `bool` | `True` when currently connected. |
| `tcp.Connected` | `Callable[[TcpSocketHandle], None]` | Callback invoked on connection established. |
| `tcp.Data` | `Callable[[TcpSocketHandle], None]` | Callback invoked when bytes arrive in the buffer. |
| `tcp.Closed` | `Callable[[TcpSocketHandle], None]` | Callback invoked when the remote peer closes the connection. |
| `tcp.Error` | `Callable[[TcpSocketHandle, str], None]` | Callback invoked on socket error `(sock, err)`. |
| `tcp.Timeout` | `Callable[[TcpSocketHandle], None]` | Callback invoked on read/write timeout. |

---

### 8.4 `Controls` & `Properties` Global Tables

In addition to `self.<ControlName>` and `self.<PropertyName>`, you can directly index the global `Controls` and `Properties` dictionaries at runtime (useful in `TextController` or procedural scripts):
- `Controls["Gain"].Value`
- `Properties["Port"].Value`

---

### 8.5 `qsys.lua()` Raw Lua Escape Hatch

If you ever need to emit a raw Lua 5.3 snippet directly into the generated output:
```python
qsys.lua("-- Raw Lua 5.3 injected verbatim\ncollectgarbage()")
```

---

### 8.6 Built-in Q-SYS Core Libraries

The following Q-SYS Core global libraries are pre-registered in the QSysPy runtime environment and can be called directly without warnings:

| Library | Description | Common Usage |
| :--- | :--- | :--- |
| `SerialPort` | Q-SYS RS-232/422/485 serial port API | `SerialPort.New(1)` |
| `HttpClient` | HTTP/HTTPS client & URL helpers | `HttpClient.Download({ Url = "..." })`, `HttpClient.Upload(...)` |
| `NamedControl` | Read/write Named Controls in the design | `NamedControl.GetValue("MasterGain")`, `NamedControl.SetValue(...)` |
| `Notifications` | Publish/subscribe inter-script messaging | `Notifications.Subscribe(...)`, `Notifications.Publish(...)` |
| `Component` | Access Named Components in the design | `Component.New("MyMixer")` |
| `Log` | Q-SYS Core Event Log | `Log.Message("Started")`, `Log.Error("Fault")` |
| `System` | Q-SYS Core system info & version | `System.IsEmulating`, `System.Version` |
| `Crypto` | Hashing, Base64, HMAC, and ciphers | `Crypto.Base64Encode(data)`, `Crypto.MD5Compute(data)` |
| `rapidjson` / `json` | Built-in JSON encode/decode | `rapidjson.encode(obj)`, `rapidjson.decode(text)` |

---

## 9. Lua 5.3 Standard Library Modules

All standard Lua 5.3 libraries allowed in the Q-SYS sandbox are typed and available in QSysPy:

### `math`
- **Constants**: `math.pi`, `math.huge`
- **Functions**: `math.floor(x)`, `math.ceil(x)`, `math.sqrt(x)`, `math.abs(x)`, `math.exp(x)`, `math.log(x)`, `math.sin(x)`, `math.cos(x)`, `math.tan(x)`, `math.fmod(x, y)`, `math.min(...)`, `math.max(...)`, `math.random([m, [n]])`, `math.randomseed(seed)`

### `string`
- **Functions**: `string.char(...)`, `string.byte(s, [i])`, `string.len(s)`, `string.rep(s, n)`, `string.format(fmt, ...)`, `string.sub(s, i, [j])`, `string.upper(s)`, `string.lower(s)`, `string.find(s, pattern)`, `string.gsub(s, pattern, repl)`, `string.match(s, pattern)`, `string.reverse(s)`

### `table`
- **Functions**: `table.insert(t, value)`, `table.remove(t, [pos])`, `table.concat(t, [sep])`, `table.sort(t, [comp])`, `table.unpack(t)`

### `bit32`
- **Functions**: `bit32.band(a, b)`, `bit32.bor(a, b)`, `bit32.bxor(a, b)`, `bit32.bnot(a)`, `bit32.lshift(a, b)`, `bit32.rshift(a, b)`

### Global Lua Functions
- `tonumber(value, [base])`, `tostring(value)`, `type(value)`, `ipairs(t)`, `pairs(t)`, `next(t, [key])`, `unpack(t)`, `pcall(f, ...)`, `error(message, [level])`, `assert(value, [message])`

---

## 10. Python Built-ins & Language Features

### Python Built-in Functions & How They Lower to Lua

| Python Built-in | Compiled Lua 5.3 Output | Notes |
| :--- | :--- | :--- |
| `print(*args)` | `print(...)` | Prints to the Q-SYS debug console. |
| `len(x)` | `#x` | Length of string, list, or table. |
| `str(x)` | `tostring(x)` | Converts value to string (also folded at compile time in static expressions). |
| `int(x)` | `tonumber(x)` | Converts string/number to numeric value. |
| `float(x)` | `tonumber(x)` | Converts string/number to float. |
| `bool(x)` | Boolean coercion | Converts value to boolean. |
| `range(stop)` / `range(start, stop, [step])` | Numeric `for i = start, stop - 1, step do` | Zero-allocation numeric loop in `for` statements and compile-time unrolling in list comprehensions. |
| `abs(x)` | `math.abs(x)` | Absolute value. |
| `min(*args)` | `math.min(...)` | Minimum value. |
| `max(*args)` | `math.max(...)` | Maximum value. |
| `chr(code)` | `string.char(code)` | Byte value (`0..255`) to 1-byte string. |
| `ord(ch)` | `string.byte(ch)` | 1-byte string to integer byte value. |
| `super()` | Base class metatable call | Calls inherited methods (`super().__init__(...)`). |

*(Note: Dynamic/OS Python built-ins unavailable on Q-SYS Core—`eval`, `exec`, `compile`, `globals`, `locals`, `vars`, `__import__`, `input`, `open`—produce clear, descriptive compile-time diagnostics.)*

### Supported Python Language Features
- **Variables & Scoping**: Automatic `local` declaration tracking across function, class, and block scopes.
- **Strings & f-Strings**: Standard strings and f-strings (`f"Host {ip}:{port}"` $\rightarrow$ `tostring(...)` concatenation).
- **Operators**:
  - Arithmetic: `+`, `-`, `*`, `/`, `//` (floor division), `%`, `**` (`^` in Lua)
  - Bitwise: `&`, `|`, `^` (`~` in Lua 5.3), `~`, `<<`, `>>`
  - Comparisons: `==`, `!=` (`~=` in Lua), `<`, `<=`, `>`, `>=`
  - Boolean: `and`, `or`, `not`
  - Augmented assignments: `+=`, `-=`, `*=`, `/=`
- **Control Flow**: `if` / `elif` / `else`, `while`, `for i in range(...)`, `for item in list:`, `break`, `continue` (lowered via Lua 5.3 `::continue::` labels).
- **Functions & Lambdas**: Default parameter values, multiple return values (`return a, b`), `lambda x: ...`.
- **Classes & Single Inheritance**: Custom helper classes with `__init__`, instance methods, class attributes, single inheritance (`class Child(Parent):`), and `super().method(...)`.
- **Multi-File Module Imports**: `import utils` and `from helpers import PacketBuilder` bundle cleanly into a single self-contained `.qplug` or `.lua` output file.

---

## 11. End-to-End Examples

### Example 1: Multi-Channel DSP Amplifier Plugin (`QSysPlugin` with Tabs & `count=4`)

```python
from qsys import QSysPlugin, Control, Property, UI, Timer, TcpSocket


class ProAmpPlugin(QSysPlugin):
    name = "Acme~ProAmp 4x500"
    version = "1.2.0"
    id = "7f8e9d0c-1b2a-4c3d-8e9f-0a1b2c3d4e5f"
    author = "Acme Audio"
    manufacturer = "Acme"
    description = "4-Channel Network DSP Amplifier"
    show_debug = True

    # Design-Time Properties (Inspector Panel)
    Port = Property.Integer(default=4001, min=1, max=65535, description="TCP control port")
    PollRate = Property.Float(default=2.0, min=0.5, max=30.0, description="Status poll interval (s)")
    Protocol = Property.Enum(choices=["ASCII", "Binary"], default="ASCII")

    # Connection Controls
    IPAddress = Control.IPAddress(default="192.168.1.100")
    Connect = Control.Boolean(type="Toggle")
    Online = Control.Indicator(pins="Output")
    Status = Control.Text(default="Disconnected", type="ReadOnly", pins="Output")

    # 4-Channel Controls (count=4)
    Signal = Control.Indicator(count=4, pins="Output")
    Gain = Control.Fader(min=-80.0, max=12.0, default=0.0, unit="dB", count=4)
    Mute = Control.Boolean(type="Toggle", count=4)

    # Multi-Page Declarative UI
    pages = [
        UI.Page(
            "Mixer",
            UI.Header("ProAmp 4x500 — Channel Strips"),
            UI.GroupBox(
                "Channels",
                UI.Row(
                    *[
                        UI.Col(
                            UI.Field(f"Sig {i + 1}", Signal[i], color="Green"),
                            UI.Field(f"Ch {i + 1}", Gain[i], style="Fader", color="#3b82f6"),
                            UI.Field(f"Mute {i + 1}", Mute[i], color="Red"),
                        )
                        for i in range(4)
                    ],
                    gap=14,
                ),
            ),
        ),
        UI.Page(
            "Setup",
            UI.Header("Network Configuration"),
            UI.GroupBox(
                "Connection",
                UI.Row(
                    UI.Field("Online", Online, color="Green"),
                    UI.Field("Connect", Connect, color="Blue"),
                ),
                UI.Field("Device IP", IPAddress, label_pos="left"),
                UI.Field("Status", Status, label_pos="left", width=200),
            ),
        ),
    ]

    def __init__(self):
        self.sock = TcpSocket.New()
        self.heartbeat = Timer.New()
        self.heartbeat.EventHandler = self.on_heartbeat

    @Connect.EventHandler
    def on_connect_toggle(self, ctl):
        if ctl.Boolean:
            self.Status.String = f"Connecting to {self.IPAddress.String}:{self.Port}..."
            self.sock.Connect(self.IPAddress.String, self.Port)
            self.Online.Boolean = True
            self.heartbeat.Start(self.PollRate)
        else:
            self.heartbeat.Stop()
            self.sock.Disconnect()
            self.Online.Boolean = False
            self.Status.String = "Disconnected"

    @Gain.EventHandler
    def on_gain_changed(self, ctl, index):
        # `index` is 0-based (0..3)
        ch = index + 1
        self.sock.Write(f"SET GAIN {ch} {ctl.Value}\r\n")

    @Mute.EventHandler
    def on_mute_changed(self, ctl, index):
        ch = index + 1
        state = 1 if ctl.Boolean else 0
        self.sock.Write(f"SET MUTE {ch} {state}\r\n")

    def on_heartbeat(self, timer):
        if self.Online.Boolean:
            self.sock.Write("PING\r\n")
```

---

### Example 2: Lightweight Text Controller Script (`TextController`)

```python
from qsys import TextController, Control, Timer


class RoomScheduler(TextController):
    PowerOn = Control.Button(type="Trigger")
    PowerOff = Control.Button(type="Trigger")
    SystemState = Control.Text(default="Standby", type="ReadOnly")
    WarmupProgress = Control.Knob(min=0.0, max=100.0, default=0.0, unit="Percent")

    def __init__(self):
        self.timer = Timer.New()
        self.timer.EventHandler = self.on_step
        self.SystemState.String = "Ready"

    @PowerOn.EventHandler
    def on_power_on(self, ctl):
        self.SystemState.String = "Warming Up..."
        self.WarmupProgress.Value = 0.0
        self.timer.Start(0.5)

    @PowerOff.EventHandler
    def on_power_off(self, ctl):
        self.timer.Stop()
        self.WarmupProgress.Value = 0.0
        self.SystemState.String = "Standby"

    def on_step(self, t):
        next_val = self.WarmupProgress.Value + 20.0
        if next_val >= 100.0:
            self.WarmupProgress.Value = 100.0
            self.SystemState.String = "Active"
            self.timer.Stop()
        else:
            self.WarmupProgress.Value = next_val
```
