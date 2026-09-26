pub mod controls;
pub mod layout;
pub mod plugin_info;
pub mod properties;

// Public re-exports for clean downstream usage
pub use controls::{
    ButtonType, ControlDef, ControlKind, ControlMember, ControlUnit, PinStyle, TextType,
};
pub use layout::{
    Color, ControlLayoutDef, ControlStyle, GraphicItem, GraphicItemKind, Position, Size,
};
pub use plugin_info::PluginInfo;
pub use properties::{PropertyDef, PropertyKind};

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
