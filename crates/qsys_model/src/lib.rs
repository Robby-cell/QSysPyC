pub mod controls;
pub mod layout;
pub mod plugin_info;
pub mod properties;

// Public re-exports for clean downstream usage
pub use controls::{
    ButtonType, ControlDef, ControlKind, ControlMember, ControlUnit, GenericControlSpec, PinStyle,
    TextType,
};
pub use layout::{
    Color, ControlLayoutDef, ControlStyle, DynAxisExpr, DynLinearSum, DynLinearTerm,
    DynamicRepeatSpec, FlexDirection, GraphicItem, GraphicItemKind, GridCellPreview,
    LabelPlacement, PageLayout, Position, Size, UiControlSpec, UiGridSpec, UiNode, UiPageSpec,
};
pub use plugin_info::PluginInfo;
pub use properties::{PropertyDef, PropertyKind};

/// Plugin methods that compile to Q-SYS design-time lifecycle functions
/// (`GetControls`, `GetControlLayout`, ...) instead of runtime code.
pub const DESIGN_TIME_METHODS: &[&str] = &[
    "get_properties",
    "get_controls",
    "get_control_layout",
    "get_pages",
    "get_color",
    "get_pretty_name",
    "rectify_properties",
    "get_components",
    "get_pins",
    "get_wiring",
];

pub fn is_design_time_method(name: &str) -> bool {
    DESIGN_TIME_METHODS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_control_def_builders() {
        let fader = ControlDef::fader("MasterGain", ControlUnit::Decibels, -100.0, 20.0, 0.0)
            .with_pin_style(PinStyle::Both)
            .with_user_pin(true);

        assert_eq!(fader.name, "MasterGain");
        assert_eq!(fader.kind.qsys_type_str(), "Fader");
        assert_eq!(fader.pin_style, PinStyle::Both);
    }

    #[test]
    fn test_control_member_from_ident() {
        assert_eq!(
            ControlMember::from_ident("Value"),
            Some(ControlMember::Value)
        );
        assert_eq!(
            ControlMember::from_ident("Boolean"),
            Some(ControlMember::Boolean)
        );
        assert_eq!(ControlMember::from_ident("Valu"), None);
    }

    #[test]
    fn test_property_def_enum() {
        let prop = PropertyDef::enumeration("BaudRate", ["9600", "19200", "115200"], "9600");
        assert_eq!(prop.kind.qsys_type_name(), "enum");
    }
}
