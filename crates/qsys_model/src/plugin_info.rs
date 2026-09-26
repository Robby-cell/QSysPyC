use serde::{Deserialize, Serialize};

/// The PluginInfo table defining metadata for a Q-SYS plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub id: String, // Unique GUID/UUID identifying the plugin in Q-SYS Designer
    pub author: String,
    pub description: Option<String>,
    pub manufacturer: Option<String>,
    pub show_debug: bool,
}

impl Default for PluginInfo {
    fn default() -> Self {
        Self {
            name: "Untitled Plugin".to_string(),
            version: "0.0.0.1".to_string(),
            id: String::new(),
            author: "Unknown".to_string(),
            description: None,
            manufacturer: None,
            show_debug: false,
        }
    }
}

impl PluginInfo {
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        author: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            author: author.into(),
            ..Default::default()
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn with_manufacturer(mut self, manufacturer: impl Into<String>) -> Self {
        self.manufacturer = Some(manufacturer.into());
        self
    }

    pub fn with_show_debug(mut self, show_debug: bool) -> Self {
        self.show_debug = show_debug;
        self
    }
}
