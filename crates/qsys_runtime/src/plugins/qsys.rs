use crate::defs::{Field, Function, Global, ObjectType, Param, TypeRef};
use crate::env::{EnvBuilder, RuntimePlugin};

/// The Q-SYS Core runtime: `Controls`, `Properties`, timers, sockets and Q-SYS libraries.
pub struct QSysRuntime;

impl QSysRuntime {
    pub const NAME: &'static str = "qsys";
}

/// Q-SYS libraries that are exposed as globals but not (yet) modelled member by member.
/// Their members are accepted without diagnostics and invoked with `.`.
const OPAQUE_LIBRARIES: &[(&str, &str)] = &[
    ("SerialPort", "Q-SYS serial port API."),
    ("HttpClient", "Q-SYS HTTP/HTTPS client library."),
    (
        "NamedControl",
        "Access to named controls in the Q-SYS design.",
    ),
    (
        "Notifications",
        "Q-SYS publish/subscribe notification library.",
    ),
    (
        "Component",
        "Access to named components in the Q-SYS design.",
    ),
    ("Log", "Q-SYS Core event log."),
    ("System", "Q-SYS Core system information."),
    ("Crypto", "Q-SYS cryptography and encoding library."),
    (
        "rapidjson",
        "JSON encoding/decoding library bundled with Q-SYS.",
    ),
    ("json", "JSON encoding/decoding library bundled with Q-SYS."),
];

impl RuntimePlugin for QSysRuntime {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn install(&self, env: &mut EnvBuilder) {
        install_design_access(env);
        install_timer(env);
        install_udp(env);
        install_tcp(env);
        install_ui(env);

        for (name, doc) in OPAQUE_LIBRARIES {
            env.add_type(ObjectType::library(*name).doc(*doc));
            env.add_global(Global::object(*name).doc(*doc));
        }

        env.remove("input", "Q-SYS scripts have no standard input");
        env.remove(
            "open",
            "Python file objects are not supported on the Q-SYS target",
        );
    }
}

fn install_design_access(env: &mut EnvBuilder) {
    env.add_global(
        Global::value(
            "Controls",
            TypeRef::dict(TypeRef::Str, TypeRef::object("ControlHandle")),
        )
        .doc("Table of the component's controls, indexed by control name (`Controls[\"Gain\"]`).")
        .runtime_only(),
    );
    env.add_global(
        Global::value(
            "Properties",
            TypeRef::dict(TypeRef::Str, TypeRef::object("PropertyHandle")),
        )
        .doc("Table of the component's properties, indexed by property name (`Properties[\"Port\"].Value`).")
        .runtime_only(),
    );

    env.add_type(
        ObjectType::library("QSysNamespace")
            .doc("Built-in compiler escape hatches and runtime utilities.")
            .method(
                Function::new("lua")
                    .arg(Param::new("code", TypeRef::Str).doc("Raw Lua 5.3 string literal."))
                    .doc("Escape hatch: injects raw, uncompiled Lua 5.3 code directly into the compiled output.")
                    .snippet("lua(\"${1:print('hello from lua')}\")"),
            ),
    );
    env.add_global(
        Global::value("qsys", TypeRef::object("QSysNamespace"))
            .doc("Compiler escape hatches (`qsys.lua(\"...\")`)."),
    );
}

fn install_timer(env: &mut EnvBuilder) {
    env.add_type(
        ObjectType::handle("TimerHandle")
            .doc("Active Q-SYS event timer.")
            .field(
                Field::new(
                    "EventHandler",
                    TypeRef::callable([TypeRef::object("TimerHandle")]),
                )
                .doc("Callback invoked every time the timer period elapses."),
            )
            .method(
                Function::new("Start")
                    .arg(Param::new("seconds", TypeRef::Float).doc("Timer interval in seconds."))
                    .doc("Starts or restarts the timer with the given interval in seconds.")
                    .snippet("Start(${1:1.0})"),
            )
            .method(Function::new("Stop").doc("Stops the timer."))
            .method(Function::new("Reset").doc("Resets the timer counter.")),
    );
    env.add_type(
        ObjectType::library("Timer")
            .doc("Q-SYS native event loop timer. Instantiate via `Timer.New()`.")
            .static_method(
                Function::new("New")
                    .returns(TypeRef::object("TimerHandle"))
                    .doc("Creates a new timer instance."),
            )
            .static_method(
                Function::new("CallAfter")
                    .arg(Param::new("callback", TypeRef::callable([])).doc("Function to call."))
                    .arg(Param::new("delay", TypeRef::Float).doc("Delay in seconds."))
                    .doc("Calls `callback` once after `delay` seconds."),
            ),
    );
    env.add_global(
        Global::object("Timer")
            .doc("Q-SYS native event loop timer. Instantiate via `Timer.New()`."),
    );
}

fn install_udp(env: &mut EnvBuilder) {
    env.add_type(
        ObjectType::library("UdpPacket")
            .doc("Incoming UDP packet received by a UdpSocket.")
            .field(Field::new("Address", TypeRef::Str).doc("Sender IP address."))
            .field(Field::new("Port", TypeRef::Int).doc("Sender UDP port."))
            .field(Field::new("Data", TypeRef::Str).doc("Packet payload.")),
    );
    env.add_type(
        ObjectType::handle("UdpSocketHandle")
            .doc("Active UDP socket handle.")
            .field(
                Field::new(
                    "Data",
                    TypeRef::callable([
                        TypeRef::object("UdpSocketHandle"),
                        TypeRef::object("UdpPacket"),
                    ]),
                )
                .doc("Callback assigned to receive incoming UDP packets."),
            )
            .field(
                Field::new(
                    "Closed",
                    TypeRef::callable([TypeRef::object("UdpSocketHandle")]),
                )
                .doc("Callback invoked when the socket closes."),
            )
            .method(
                Function::new("Open")
                    .arg(
                        Param::new("ip", TypeRef::Str)
                            .optional()
                            .doc("Optional local interface IP to bind."),
                    )
                    .arg(
                        Param::new("port", TypeRef::Int)
                            .optional()
                            .doc("Optional local UDP port to bind."),
                    )
                    .doc("Binds and opens the UDP socket."),
            )
            .method(Function::new("Close").doc("Closes the UDP socket."))
            .method(
                Function::new("Send")
                    .arg(
                        Param::new("ip", TypeRef::Str)
                            .doc("Destination IPv4/IPv6 or broadcast address."),
                    )
                    .arg(
                        Param::new("port", TypeRef::Int)
                            .doc("Destination UDP port number (1-65535)."),
                    )
                    .arg(Param::new("data", TypeRef::Str).doc("Binary or text payload to send."))
                    .doc("Transmits a UDP datagram to the specified destination.")
                    .snippet("Send(${1:self.IPAddress.String}, ${2:9}, ${3:packet})"),
            )
            .method(
                Function::new("JoinMulticast")
                    .arg(Param::new("address", TypeRef::Str).doc("Multicast group address."))
                    .doc("Joins an IP multicast group.")
                    .snippet("JoinMulticast(${1:\"224.0.0.1\"})"),
            )
            .method(
                Function::new("LeaveMulticast")
                    .arg(Param::new("address", TypeRef::Str).doc("Multicast group address."))
                    .doc("Leaves an IP multicast group."),
            ),
    );
    env.add_type(
        ObjectType::library("UdpSocket")
            .doc("Q-SYS native UDP networking class. Instantiate via `UdpSocket.New()`.")
            .static_method(
                Function::new("New")
                    .returns(TypeRef::object("UdpSocketHandle"))
                    .doc("Creates a new UDP socket."),
            ),
    );
    env.add_global(
        Global::object("UdpSocket")
            .doc("Q-SYS native UDP networking class. Instantiate via `UdpSocket.New()`."),
    );
}

fn install_tcp(env: &mut EnvBuilder) {
    let this = || TypeRef::object("TcpSocketHandle");
    env.add_type(
        ObjectType::handle("TcpSocketHandle")
            .doc("Active TCP socket handle.")
            .field(
                Field::new("IsConnected", TypeRef::Bool)
                    .doc("True when the TCP socket is currently connected."),
            )
            .field(
                Field::new("Connected", TypeRef::callable([this()]))
                    .doc("Callback invoked when the connection is established."),
            )
            .field(
                Field::new("Data", TypeRef::callable([this()]))
                    .doc("Callback invoked when data arrives."),
            )
            .field(
                Field::new("Closed", TypeRef::callable([this()]))
                    .doc("Callback invoked when the remote end closes the connection."),
            )
            .field(
                Field::new("Error", TypeRef::callable([this(), TypeRef::Str]))
                    .doc("Callback invoked on socket errors."),
            )
            .field(
                Field::new("Timeout", TypeRef::callable([this()]))
                    .doc("Callback invoked when a read or write times out."),
            )
            .method(
                Function::new("Connect")
                    .arg(Param::new("ip", TypeRef::Str).doc("Remote host IP or hostname."))
                    .arg(Param::new("port", TypeRef::Int).doc("Remote TCP port."))
                    .doc("Opens a TCP connection to the host and port.")
                    .snippet("Connect(${1:ip}, ${2:port})"),
            )
            .method(Function::new("Disconnect").doc("Closes the active TCP connection."))
            .method(
                Function::new("Write")
                    .arg(Param::new("data", TypeRef::Str).doc("String or binary payload to transmit."))
                    .doc("Writes data to the connected TCP socket.")
                    .snippet("Write(${1:data})"),
            )
            .method(
                Function::new("Read")
                    .arg(
                        Param::new("length", TypeRef::Int)
                            .optional()
                            .doc("Maximum number of bytes to read."),
                    )
                    .returns(TypeRef::Str)
                    .doc("Reads up to `length` bytes from the receive buffer.")
                    .snippet("Read(${1:1024})"),
            )
            .method(
                Function::new("ReadLine")
                    .arg(
                        Param::new("eol", TypeRef::Any)
                            .optional()
                            .doc("End-of-line mode, e.g. `TcpSocket.EOL.Any`."),
                    )
                    .returns(TypeRef::Any)
                    .doc("Reads a single line from the receive buffer, or `nil` if no full line is available.")
                    .snippet("ReadLine(${1:TcpSocket.EOL.Any})"),
            ),
    );
    env.add_type(
        ObjectType::library("TcpSocket")
            .doc("Q-SYS native TCP client networking class. Instantiate via `TcpSocket.New()`.")
            .static_method(
                Function::new("New")
                    .returns(this())
                    .doc("Creates a new TCP socket."),
            ),
    );
    env.add_global(
        Global::object("TcpSocket")
            .doc("Q-SYS native TCP client networking class. Instantiate via `TcpSocket.New()`."),
    );
}

fn install_ui(env: &mut EnvBuilder) {
    let ui_node = || TypeRef::object("UINode");
    let ctl_handle = || TypeRef::object("ControlHandle");

    env.add_type(
        ObjectType::library("UINode")
            .doc("Declarative Q-SYS UI layout node expanded into `GetControlLayout` and `GetPages` tables."),
    );

    env.add_type(
        ObjectType::library("UI")
            .doc("Declarative Flexbox-style UI layout builder for Q-SYS `GetControlLayout(props)` and `GetPages(props)`.")
            .static_method(
                Function::new("Page")
                    .arg(
                        Param::new("title", TypeRef::Str)
                            .optional()
                            .doc("Tab title in Q-SYS `GetPages` (defaults to `\"Main\"`)."),
                    )
                    .variadic()
                    .returns(ui_node())
                    .doc("Declares a Q-SYS Plugin UI page (`GetPages` + `GetControlLayout`). Automatically solves child positions and sizes.")
                    .snippet("Page(\n    \"${1:Main}\",\n    ${0}\n)"),
            )
            .static_method(
                Function::new("Pages")
                    .variadic()
                    .returns(TypeRef::list(ui_node()))
                    .doc("Groups multiple `UI.Page(...)` definitions into a multi-page Q-SYS plugin interface.")
                    .snippet("Pages(\n    UI.Page(\"${1:Main}\", ${0}),\n)"),
            )
            .static_method(
                Function::new("GroupBox")
                    .arg(
                        Param::new("title", TypeRef::Str)
                            .optional()
                            .doc("Header text displayed on the GroupBox border."),
                    )
                    .variadic()
                    .returns(ui_node())
                    .doc("Creates a Q-SYS `GroupBox` container that automatically sizes and positions its child controls and rows.")
                    .snippet("GroupBox(\n    \"${1:Controls}\",\n    ${0}\n)"),
            )
            .static_method(
                Function::new("Section")
                    .arg(
                        Param::new("title", TypeRef::Str)
                            .optional()
                            .doc("Header text displayed on the section border."),
                    )
                    .variadic()
                    .returns(ui_node())
                    .doc("Alias for `UI.GroupBox`: creates a framed Q-SYS section container.")
                    .snippet("Section(\n    \"${1:Section}\",\n    ${0}\n)"),
            )
            .static_method(
                Function::new("Row")
                    .variadic()
                    .returns(ui_node())
                    .doc("Horizontal Flexbox-style row container (`UI.Row(UI.Field(...), UI.Field(...), gap=8)`).")
                    .snippet("Row(\n    ${0}\n)"),
            )
            .static_method(
                Function::new("Col")
                    .variadic()
                    .returns(ui_node())
                    .doc("Vertical Flexbox-style column container (`UI.Col(UI.Field(...), UI.Field(...), gap=8)`).")
                    .snippet("Col(\n    ${0}\n)"),
            )
            .static_method(
                Function::new("Field")
                    .arg(
                        Param::new("label", TypeRef::Str)
                            .doc("Text label graphic rendered above or beside the control."),
                    )
                    .arg(
                        Param::new("control", ctl_handle())
                            .doc("Declared control reference (`self.Gain`, `Gain`, or `ChGain[0]`)."),
                    )
                    .arg(
                        Param::new("style", TypeRef::Str)
                            .optional()
                            .doc("Optional Q-SYS widget style override (`\"Knob\"`, `\"Fader\"`, `\"Button\"`, `\"Text\"`, `\"Led\"`, `\"Meter\"`, `\"ComboBox\"`)."),
                    )
                    .arg(
                        Param::new("pretty_name", TypeRef::Str)
                            .optional()
                            .doc("Optional Q-SYS `PrettyName` override for pin exposure (`\"Group~Name\"` or `\"Ch {i} Gain\"`)."),
                    )
                    .arg(
                        Param::new("labels", TypeRef::List(Box::new(TypeRef::Str)))
                            .optional()
                            .doc("Optional per-channel label list when `count > 1` (e.g. `[\"Left\", \"Right\"]`)."),
                    )
                    .arg(
                        Param::new("direction", TypeRef::Str)
                            .optional()
                            .doc("Layout direction when expanding a `count > 1` control (`\"row\"` or `\"col\"`)."),
                    )
                    .arg(
                        Param::new("width", TypeRef::Float)
                            .optional()
                            .doc("Optional explicit widget width in pixels."),
                    )
                    .arg(
                        Param::new("height", TypeRef::Float)
                            .optional()
                            .doc("Optional explicit widget height in pixels."),
                    )
                    .arg(
                        Param::new("color", TypeRef::Str)
                            .optional()
                            .doc("Optional hex color (`\"#007acc\"`) or named color."),
                    )
                    .returns(ui_node())
                    .doc("Pairs a Q-SYS `Label` graphic with a declared `ControlHandle` (`self.Gain`, `Gain`, or `self.Gain[0]`) and sets `PrettyName` so pins can be exposed. For `count > 1`, auto-expands all channels (supports `{i}` in label, `labels=[...]`, and `direction=\"row\"`).")
                    .snippet("Field(\"${1:Label}\", ${2:self.ControlName})"),
            )
            .static_method(
                Function::new("Control")
                    .arg(
                        Param::new("control", ctl_handle())
                            .doc("Declared control reference (`self.Gain`, `Gain`, or `ChGain[0]`). Multi-channel controls (`count > 1`) auto-expand when unindexed."),
                    )
                    .arg(
                        Param::new("style", TypeRef::Str)
                            .optional()
                            .doc("Optional Q-SYS widget style override (`\"Knob\"`, `\"Fader\"`, `\"Button\"`, `\"Text\"`, `\"Led\"`, `\"Meter\"`)."),
                    )
                    .arg(
                        Param::new("pretty_name", TypeRef::Str)
                            .optional()
                            .doc("Optional Q-SYS `PrettyName` override for pin exposure (`\"Group~Name\"` or `\"Ch {i}\"`)."),
                    )
                    .arg(
                        Param::new("label", TypeRef::Str)
                            .optional()
                            .doc("Optional label graphic rendered above or beside the control."),
                    )
                    .arg(
                        Param::new("direction", TypeRef::Str)
                            .optional()
                            .doc("Layout direction when expanding a `count > 1` control (`\"row\"` or `\"col\"`)."),
                    )
                    .arg(
                        Param::new("width", TypeRef::Float)
                            .optional()
                            .doc("Optional explicit widget width in pixels."),
                    )
                    .arg(
                        Param::new("height", TypeRef::Float)
                            .optional()
                            .doc("Optional explicit widget height in pixels."),
                    )
                    .arg(
                        Param::new("color", TypeRef::Str)
                            .optional()
                            .doc("Optional hex color (`\"#007acc\"`) or named color."),
                    )
                    .returns(ui_node())
                    .doc("Places a declared `ControlHandle` (`self.Gain`, `Gain`, or `ChGain[0]`) in the UI layout with an automatic `PrettyName` for pin exposure.")
                    .snippet("Control(${1:self.ControlName})"),
            )
            .static_method(
                Function::new("Header")
                    .arg(Param::new("text", TypeRef::Str).doc("Header banner text."))
                    .arg(
                        Param::new("width", TypeRef::Float)
                            .optional()
                            .doc("Optional explicit width in pixels."),
                    )
                    .arg(
                        Param::new("height", TypeRef::Float)
                            .optional()
                            .doc("Header height in pixels (defaults to 28)."),
                    )
                    .returns(ui_node())
                    .doc("Emits a Q-SYS `Header` section bar graphic.")
                    .snippet("Header(\"${1:Section Title}\")"),
            )
            .static_method(
                Function::new("Label")
                    .arg(Param::new("text", TypeRef::Str).doc("Static label text."))
                    .arg(
                        Param::new("width", TypeRef::Float)
                            .optional()
                            .doc("Optional explicit width in pixels."),
                    )
                    .arg(
                        Param::new("height", TypeRef::Float)
                            .optional()
                            .doc("Label height in pixels (defaults to 22)."),
                    )
                    .returns(ui_node())
                    .doc("Emits a static Q-SYS `Label` text graphic.")
                    .snippet("Label(\"${1:Label Text}\")"),
            )
            .static_method(
                Function::new("Spacer")
                    .arg(
                        Param::new("width", TypeRef::Float)
                            .optional()
                            .doc("Horizontal space in pixels (defaults to 12)."),
                    )
                    .arg(
                        Param::new("height", TypeRef::Float)
                            .optional()
                            .doc("Vertical space in pixels (defaults to 12)."),
                    )
                    .returns(ui_node())
                    .doc("Inserts empty layout spacing inside a `UI.Row` or `UI.Col`.")
                    .snippet("Spacer(${1:12})"),
            ),
    );

    env.add_global(
        Global::object("UI")
            .doc("Declarative Flexbox-style UI layout builder for Q-SYS `GetControlLayout` and `GetPages`.")
            .design_time_only(),
    );
}
