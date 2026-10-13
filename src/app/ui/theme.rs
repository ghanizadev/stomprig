use egui::Visuals;
use egui_desktop::{ThemeMode, ThemeProvider, TitleBarTheme, detect_system_dark_mode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ThemePreference {
    Light,
    Dark,
    #[default]
    System,
}

impl ThemePreference {
    pub(crate) fn desktop_mode(self) -> ThemeMode {
        match self {
            Self::Light => ThemeMode::Light,
            Self::Dark => ThemeMode::Dark,
            Self::System => ThemeMode::System,
        }
    }

    pub(crate) fn is_dark(self) -> bool {
        match self {
            Self::Light => false,
            Self::Dark => true,
            Self::System => detect_system_dark_mode(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Theme {
    preference: ThemePreference,
    dark: bool,
    visuals: Visuals,
}

impl Theme {
    pub(crate) fn new(preference: ThemePreference) -> Self {
        let dark = preference.is_dark();
        let visuals = if dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };

        Self {
            preference,
            dark,
            visuals,
        }
    }

    pub(crate) fn preference(&self) -> ThemePreference {
        self.preference
    }

    pub(crate) fn visuals(&self) -> &Visuals {
        &self.visuals
    }

    pub(crate) fn bypass_colors(&self, effect_enabled: bool) -> (egui::Color32, egui::Color32) {
        if effect_enabled {
            (
                egui::Color32::from_rgb(218, 222, 228),
                egui::Color32::from_rgb(36, 38, 42),
            )
        } else {
            (
                egui::Color32::from_rgb(60, 64, 70),
                egui::Color32::from_rgb(226, 229, 233),
            )
        }
    }

    fn title_bar_theme(&self) -> TitleBarTheme {
        if self.dark {
            TitleBarTheme::dark()
        } else {
            TitleBarTheme::light()
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::new(ThemePreference::default())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct StompRigThemeProvider;

impl ThemeProvider for StompRigThemeProvider {
    fn get_title_bar_theme(&self, _theme_id: &str, mode: ThemeMode) -> Option<TitleBarTheme> {
        let preference = match mode {
            ThemeMode::Light => ThemePreference::Light,
            ThemeMode::Dark => ThemePreference::Dark,
            ThemeMode::System => ThemePreference::System,
        };
        Some(Theme::new(preference).title_bar_theme())
    }

    fn get_egui_visuals(&self, _theme_id: &str, mode: ThemeMode) -> Option<Visuals> {
        let preference = match mode {
            ThemeMode::Light => ThemePreference::Light,
            ThemeMode::Dark => ThemePreference::Dark,
            ThemeMode::System => ThemePreference::System,
        };
        Some(Theme::new(preference).visuals)
    }

    fn list_available_themes(&self) -> Vec<String> {
        vec!["stomprig".to_owned()]
    }
}
