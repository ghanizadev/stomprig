use egui::Color32;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ThemeColors {
    pub(crate) muted_foreground: Color32,
}

#[derive(Clone, Debug)]
pub(crate) struct Theme {
    colors: ThemeColors,
    renderer_theme: egui_shadcn::Theme,
}

impl Theme {
    pub(crate) fn colors(&self) -> ThemeColors {
        self.colors
    }

    pub(super) fn renderer_theme(&self) -> &egui_shadcn::Theme {
        &self.renderer_theme
    }
}

impl Default for Theme {
    fn default() -> Self {
        let theme = egui_shadcn::Theme::default();
        Self {
            colors: ThemeColors {
                muted_foreground: theme.palette.muted_foreground,
            },
            renderer_theme: theme,
        }
    }
}
