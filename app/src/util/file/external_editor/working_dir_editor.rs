use serde::{Deserialize, Serialize};
use warp_core::ui::icons::Icon;

#[derive(
    Default,
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    Hash,
    enum_iterator::Sequence,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Editor used when opening the terminal working directory from the prompt chip row.",
    rename_all = "snake_case"
)]
pub enum WorkingDirEditor {
    #[default]
    VsCode,
    Cursor,
    Windsurf,
    Antigravity,
    Zed,
}

impl WorkingDirEditor {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::VsCode => "VS Code",
            Self::Cursor => "Cursor",
            Self::Windsurf => "Windsurf",
            Self::Antigravity => "Antigravity",
            Self::Zed => "Zed",
        }
    }

    pub fn command(self) -> &'static str {
        match self {
            Self::VsCode => "code",
            Self::Cursor => "cursor",
            Self::Windsurf => "windsurf",
            Self::Antigravity => "antigravity",
            Self::Zed => "zed",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Self::VsCode => Icon::VsCodeLogo,
            Self::Cursor => Icon::CursorLogo,
            Self::Windsurf => Icon::WindsurfLogo,
            Self::Antigravity => Icon::AntigravityLogo,
            Self::Zed => Icon::ZedLogo,
        }
    }
}

impl std::fmt::Display for WorkingDirEditor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}
