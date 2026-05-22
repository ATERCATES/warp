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
    description = "External editor opened by the 'Open in IDE' prompt chip.",
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

enum SshArgStyle {
    VsCodeRemote,
    ZedSshUrl,
}

struct EditorMeta {
    display_name: &'static str,
    command: &'static str,
    icon: Icon,
    ssh: SshArgStyle,
}

impl WorkingDirEditor {
    fn meta(self) -> EditorMeta {
        match self {
            Self::VsCode => EditorMeta {
                display_name: "VS Code",
                command: "code",
                icon: Icon::VsCodeLogo,
                ssh: SshArgStyle::VsCodeRemote,
            },
            Self::Cursor => EditorMeta {
                display_name: "Cursor",
                command: "cursor",
                icon: Icon::CursorLogo,
                ssh: SshArgStyle::VsCodeRemote,
            },
            Self::Windsurf => EditorMeta {
                display_name: "Windsurf",
                command: "windsurf",
                icon: Icon::WindsurfLogo,
                ssh: SshArgStyle::VsCodeRemote,
            },
            Self::Antigravity => EditorMeta {
                display_name: "Antigravity",
                command: "antigravity",
                icon: Icon::AntigravityLogo,
                ssh: SshArgStyle::VsCodeRemote,
            },
            Self::Zed => EditorMeta {
                display_name: "Zed",
                command: "zed",
                icon: Icon::ZedLogo,
                ssh: SshArgStyle::ZedSshUrl,
            },
        }
    }

    pub fn display_name(self) -> &'static str {
        self.meta().display_name
    }

    pub fn command(self) -> &'static str {
        self.meta().command
    }

    pub fn icon(self) -> Icon {
        self.meta().icon
    }

    pub fn ssh_remote_args(self, ssh_host: &str, path: &str) -> Vec<String> {
        match self.meta().ssh {
            SshArgStyle::VsCodeRemote => vec![
                "--remote".into(),
                format!("ssh-remote+{ssh_host}"),
                path.into(),
            ],
            SshArgStyle::ZedSshUrl => {
                let normalized = if path.starts_with('/') {
                    path.to_string()
                } else {
                    format!("/{path}")
                };
                vec![format!("ssh://{ssh_host}{normalized}")]
            }
        }
    }
}

impl std::fmt::Display for WorkingDirEditor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}
