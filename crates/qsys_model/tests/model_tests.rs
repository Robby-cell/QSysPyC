use qsys_model::*;

#[test]
fn test_plugin_info_builder_and_defaults() {
    let info = PluginInfo::new("DSP Matrix", "2.1.0", "Engineering Team")
        .with_id("12345678-ABCD-EF01-2345-6789ABCDEF01")
        .with_description("Multi-channel matrix processor")
        .with_manufacturer("Acme Audio")
        .with_show_debug(true);

    assert_eq!(info.name, "DSP Matrix");
    assert_eq!(info.version, "2.1.0");
    assert_eq!(info.author, "Engineering Team");
    assert_eq!(info.id, "12345678-ABCD-EF01-2345-6789ABCDEF01");
    assert_eq!(
        info.description.as_deref(),
        Some("Multi-channel matrix processor")
    );
    assert_eq!(info.manufacturer.as_deref(), Some("Acme Audio"));
    assert!(info.show_debug);
}

#[test]
fn test_control_def_permutations() {
    // Toggle button
    let toggle = ControlDef::button("Mute", ButtonType::Toggle)
        .with_pin_style(PinStyle::Both)
        .with_user_pin(true);
    assert_eq!(toggle.name, "Mute");
    assert_eq!(toggle.kind.qsys_type_str(), "Button");
    assert_eq!(toggle.pin_style, PinStyle::Both);
    assert!(toggle.user_pin);

    // Continuous Fader (Decibels)
    let fader =
        ControlDef::fader("MasterVolume", ControlUnit::Decibels, -100.0, 12.0, 0.0).with_count(8);
    assert_eq!(fader.name, "MasterVolume");
    assert_eq!(fader.count, 8);
    assert_eq!(fader.kind.qsys_type_str(), "Fader");

    // ReadOnly Text Box
    let text = ControlDef::text("StatusDisplay", TextType::ReadOnly, "Ready");
    assert_eq!(text.name, "StatusDisplay");
    assert_eq!(text.kind.qsys_type_str(), "Text");

    // Frequency Knob (Hertz)
    let knob = ControlDef::knob("CutoffFreq", ControlUnit::Hertz, 20.0, 20000.0, 1000.0);
    assert_eq!(knob.kind.qsys_type_str(), "Knob");
}

#[test]
fn test_property_definitions() {
    let int_prop = PropertyDef::integer("ChannelCount", 8, Some(1), Some(64));
    assert_eq!(int_prop.name, "ChannelCount");
    assert_eq!(int_prop.kind.qsys_type_name(), "integer");

    let enum_prop = PropertyDef::enumeration("BaudRate", ["9600", "19200", "115200"], "9600");
    assert_eq!(enum_prop.name, "BaudRate");
    assert_eq!(enum_prop.kind.qsys_type_name(), "enum");

    let bool_prop = PropertyDef::boolean("EnableLogging", false)
        .with_description("Turns verbose logging on or off");
    assert_eq!(
        bool_prop.description.as_deref(),
        Some("Turns verbose logging on or off")
    );
}

#[test]
fn test_control_member_lookup() {
    // Valid runtime members
    assert_eq!(
        ControlMember::from_ident("Value"),
        Some(ControlMember::Value)
    );
    assert_eq!(
        ControlMember::from_ident("String"),
        Some(ControlMember::String)
    );
    assert_eq!(
        ControlMember::from_ident("Boolean"),
        Some(ControlMember::Boolean)
    );
    assert_eq!(
        ControlMember::from_ident("EventHandler"),
        Some(ControlMember::EventHandler)
    );
    assert_eq!(
        ControlMember::from_ident("RampTime"),
        Some(ControlMember::RampTime)
    );

    // Case sensitivity & typos
    assert_eq!(ControlMember::from_ident("value"), None);
    assert_eq!(ControlMember::from_ident("EventHandlr"), None);
    assert_eq!(ControlMember::from_ident("NonExistent"), None);

    // Member strings
    assert_eq!(ControlMember::Value.as_str(), "Value");
    assert_eq!(ControlMember::EventHandler.as_str(), "EventHandler");
}

#[test]
fn test_layout_and_graphic_definitions() {
    let pos = Position::new(10.0, 20.0);
    let size = Size::new(60.0, 120.0);
    let mut layout = ControlLayoutDef::new("Gain1", pos, size);
    layout.style = Some(ControlStyle::Fader);
    layout.color = Some(Color::hex("#00FF88"));
    layout.legend = Some("CH 1".to_string());

    assert_eq!(layout.position.x, 10.0);
    assert_eq!(layout.size.height, 120.0);
    assert_eq!(layout.style.as_ref().unwrap().as_str(), "Fader");

    let header = GraphicItem {
        kind: GraphicItemKind::Header {
            text: "Mixer Section".to_string(),
            font_size: 16,
        },
        position: Position::new(0.0, 0.0),
        size: Size::new(300.0, 40.0),
        dyn_x: None,
        dyn_y: None,
        dyn_width: None,
        dyn_height: None,
        color: Some(Color::rgb(30, 30, 30)),
        skip_lua_emit: false,
    };

    assert_eq!(header.position.x, 0.0);
}
