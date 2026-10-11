//! Build settings: how a compiled game behaves. They belong to the project (saved in the project file) and are
//! asked for in a dialog every time the game is compiled.
use crate::ui::settings_dialog::ItemSpec;

#[derive(Clone, Debug, PartialEq)]
pub struct BuildSettings {
    /// Window title; empty = the name of the executable.
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    pub vsync: bool,
    /// Frame rate cap; 0 = unlimited.
    pub fps_limit: u32,
    /// Show the frame rate in a corner of the game.
    pub show_fps: bool,
}

impl Default for BuildSettings {
    fn default() -> Self {
        BuildSettings { name: String::new(), width: 1280, height: 720, fullscreen: false, vsync: true, fps_limit: 0, show_fps: false }
    }
}

impl BuildSettings {
    /// The dialog's lines. `default_name` is shown when no name has been chosen yet.
    pub fn specs(&self, default_name: &str) -> Vec<ItemSpec> {
        let d = BuildSettings::default();
        vec![
            ItemSpec::Section("Game".into()),
            ItemSpec::Text("Title".into(), "Shown in the window's title bar.".into(), if self.name.is_empty() { default_name.to_string() } else { self.name.clone() }, default_name.to_string()),
            ItemSpec::Section("Window".into()),
            ItemSpec::Number("Width".into(), "Window width in pixels (320 - 7680).".into(), self.width as f32, d.width as f32, 320.0, 7680.0, true),
            ItemSpec::Number("Height".into(), "Window height in pixels (200 - 4320).".into(), self.height as f32, d.height as f32, 200.0, 4320.0, true),
            ItemSpec::Toggle("Start in full screen".into(), "F11 switches between full screen and a window.".into(), self.fullscreen, d.fullscreen),
            ItemSpec::Section("Performance".into()),
            ItemSpec::Toggle("VSync".into(), "Wait for the monitor refresh (smooth, no tearing).".into(), self.vsync, d.vsync),
            ItemSpec::Number("FPS limit".into(), "Frame rate cap. 0 = unlimited.".into(), self.fps_limit as f32, d.fps_limit as f32, 0.0, 1000.0, true),
            ItemSpec::Toggle("Show FPS".into(), "Frame rate counter in the corner of the game.".into(), self.show_fps, d.show_fps),
        ]
    }

    /// Reads the dialog's answers back (numbers and switches in order, then the texts).
    pub fn from_values(values: &[f32], texts: &[String]) -> BuildSettings {
        BuildSettings {
            name: texts.first().map(|t| t.trim().to_string()).unwrap_or_default(),
            width: values.first().map_or(1280.0, |v| *v) as u32,
            height: values.get(1).map_or(720.0, |v| *v) as u32,
            fullscreen: values.get(2).is_some_and(|v| *v > 0.5),
            vsync: values.get(3).is_some_and(|v| *v > 0.5),
            fps_limit: values.get(4).map_or(0.0, |v| *v) as u32,
            show_fps: values.get(5).is_some_and(|v| *v > 0.5),
        }
    }

    /// `width height fullscreen vsync fps_limit show_fps` (the name goes in its own tab-separated field).
    pub fn numbers_text(&self) -> String {
        format!("{} {} {} {} {} {}", self.width, self.height, self.fullscreen as u8, self.vsync as u8, self.fps_limit, self.show_fps as u8)
    }

    pub fn parse(name: &str, numbers: &str) -> BuildSettings {
        let d = BuildSettings::default();
        let v: Vec<u32> = numbers.split_whitespace().filter_map(|t| t.parse().ok()).collect();
        BuildSettings {
            name: name.trim().to_string(),
            width: v.first().copied().unwrap_or(d.width).clamp(320, 7680),
            height: v.get(1).copied().unwrap_or(d.height).clamp(200, 4320),
            fullscreen: v.get(2).is_some_and(|&x| x != 0),
            vsync: v.get(3).map_or(d.vsync, |&x| x != 0),
            fps_limit: v.get(4).copied().unwrap_or(0).min(1000),
            show_fps: v.get(5).is_some_and(|&x| x != 0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips_and_clamps() {
        let b = BuildSettings { name: "My Game".into(), width: 1920, height: 1080, fullscreen: true, vsync: false, fps_limit: 144, show_fps: true };
        assert_eq!(BuildSettings::parse(&b.name, &b.numbers_text()), b);
        let wild = BuildSettings::parse("x", "99999 1 0 1 5000 0");
        assert_eq!((wild.width, wild.height, wild.fps_limit), (7680, 200, 1000));
        assert_eq!(BuildSettings::parse("", ""), BuildSettings::default());
    }

    #[test]
    fn dialog_answers_map_back_onto_the_settings() {
        let b = BuildSettings { name: String::new(), width: 800, height: 600, fullscreen: true, vsync: false, fps_limit: 60, show_fps: true };
        let mut d = crate::ui::settings_dialog::SettingsDialog::new("Build", b.specs("demo"));
        let (values, texts) = (d.values(), d.texts());
        assert_eq!(texts, vec!["demo".to_string()], "an empty name shows the default one");
        let back = BuildSettings::from_values(&values, &texts);
        assert_eq!((back.width, back.height, back.fullscreen, back.vsync, back.fps_limit, back.show_fps), (800, 600, true, false, 60, true));
        d.result = None;
    }
}
