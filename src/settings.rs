//! Engine settings: they belong to the engine (this computer), not to a project. Stored as `key = value`
//! lines in `%APPDATA%\Polystation\settings.ini`.
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Wait for the monitor refresh (no tearing, but the frame rate is capped to it).
    pub vsync: bool,
    /// Frame rate cap when vsync is off; 0 = unlimited.
    pub fps_limit: u32,
    /// Interface size in percent; 0 = follow the Windows display scale. Needs a restart.
    pub ui_scale: u32,
    /// Field of view of the editor camera, degrees.
    pub editor_fov: f32,
    /// Multiplier for the editor camera's mouse look.
    pub look_sensitivity: f32,
    /// Let AI assistants connect (see the mcp module). Needs a restart.
    pub mcp_enabled: bool,
    pub mcp_port: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { vsync: false, fps_limit: 0, ui_scale: 0, editor_fov: 60.0, look_sensitivity: 1.0, mcp_enabled: true, mcp_port: crate::mcp::DEFAULT_PORT }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        base.join("Polystation").join("settings.ini")
    }

    pub fn load() -> Settings {
        std::fs::read_to_string(Self::path()).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, self.to_text()).map_err(|e| format!("could not save the settings: {e}"))
    }

    /// Unknown keys and unreadable values are ignored (the default stays).
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "vsync" => s.vsync = v == "true",
                "fps_limit" => s.fps_limit = v.parse().unwrap_or(s.fps_limit).min(1000),
                "ui_scale" => s.ui_scale = v.parse().map(|n: u32| if n == 0 { 0 } else { n.clamp(50, 300) }).unwrap_or(s.ui_scale),
                "editor_fov" => s.editor_fov = v.parse().map(|n: f32| n.clamp(30.0, 120.0)).unwrap_or(s.editor_fov),
                "look_sensitivity" => s.look_sensitivity = v.parse().map(|n: f32| n.clamp(0.1, 5.0)).unwrap_or(s.look_sensitivity),
                "mcp_enabled" => s.mcp_enabled = v == "true",
                "mcp_port" => s.mcp_port = v.parse().ok().filter(|&p: &u16| p >= 1024).unwrap_or(s.mcp_port),
                _ => {}
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        format!(
            "# Polystation engine settings (not part of any project)\nvsync = {}\nfps_limit = {}\nui_scale = {}\neditor_fov = {}\nlook_sensitivity = {}\nmcp_enabled = {}\nmcp_port = {}\n",
            self.vsync, self.fps_limit, self.ui_scale, self.editor_fov, self.look_sensitivity, self.mcp_enabled, self.mcp_port
        )
    }
}

/// Recently opened projects, newest first (a text file next to the settings).
pub fn recent_path() -> PathBuf {
    Settings::path().with_file_name("recent.txt")
}

pub fn recents() -> Vec<String> {
    std::fs::read_to_string(recent_path()).map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty() && std::path::Path::new(l).is_file()).map(String::from).collect()).unwrap_or_default()
}

/// Puts a project at the top of the recent list.
pub fn add_recent(path: &str) {
    let abs = std::path::absolute(path).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| path.to_string());
    let mut list: Vec<String> = recents().into_iter().filter(|p| !p.eq_ignore_ascii_case(&abs)).collect();
    list.insert(0, abs);
    list.truncate(12);
    if let Some(dir) = recent_path().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(recent_path(), list.join("\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let s = Settings { vsync: true, fps_limit: 240, ui_scale: 125, editor_fov: 75.0, look_sensitivity: 1.5, mcp_enabled: false, mcp_port: 50000 };
        assert_eq!(Settings::parse(&s.to_text()), s);
    }

    #[test]
    fn bad_values_keep_defaults_and_ranges_clamp() {
        let s = Settings::parse("fps_limit = lots\nui_scale = 9000\neditor_fov = 500\nmcp_port = 80\nwhat = 1\n");
        assert_eq!(s.fps_limit, 0);
        assert_eq!(s.ui_scale, 300);
        assert_eq!(s.editor_fov, 120.0);
        assert_eq!(s.mcp_port, crate::mcp::DEFAULT_PORT);
    }
}
