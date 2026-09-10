//! RusTXT's existing GTK visual language, expressed with Iced styles.
use iced::{
    widget::{button, container, text_input},
    Background, Border, Color, Shadow, Theme, Vector,
};
use rustxt_core::config::Palette;

#[derive(Clone, Copy)]
pub struct Colors {
    pub background: Color,
    pub foreground: Color,
    pub chrome: Color,
    pub menu: Color,
    pub muted: Color,
    pub border: Color,
    pub selection: Color,
    pub tab: Color,
    pub control: Color,
}

fn blend(a: Color, b: Color, amount: f32) -> Color {
    Color::from_rgb(
        a.r + (b.r - a.r) * amount,
        a.g + (b.g - a.g) * amount,
        a.b + (b.b - a.b) * amount,
    )
}

impl Colors {
    pub fn new(theme: &Theme, palette: Option<&Palette>) -> Self {
        let dark = theme.palette().background.r < 0.5;
        let base = if dark {
            Color::from_rgb8(30, 30, 30)
        } else {
            Color::WHITE
        };
        let foreground = if dark {
            Color::from_rgb8(238, 238, 238)
        } else {
            Color::from_rgb8(46, 46, 46)
        };
        let parse = |value: Option<&str>, fallback| {
            value
                .and_then(|value| value.parse().ok())
                .unwrap_or(fallback)
        };
        let background = parse(palette.map(|p| p.background.as_str()), base);
        let foreground = parse(palette.map(|p| p.foreground.as_str()), foreground);
        let chrome = parse(
            palette.and_then(|p| p.chrome.as_deref()),
            if palette.is_some() {
                blend(background, Color::BLACK, if dark { 0.18 } else { 0.05 })
            } else if dark {
                Color::from_rgb8(36, 36, 36)
            } else {
                Color::from_rgb8(250, 250, 250)
            },
        );
        Self {
            background,
            foreground,
            chrome,
            menu: parse(
                palette.and_then(|p| p.menu.as_deref()),
                blend(background, foreground, 0.08),
            ),
            muted: parse(
                palette.and_then(|p| p.muted.as_deref()),
                blend(foreground, background, 0.3),
            ),
            border: parse(
                palette.and_then(|p| p.border.as_deref()),
                blend(chrome, foreground, 0.1),
            ),
            selection: parse(
                palette.and_then(|p| p.selection.as_deref()),
                blend(background, theme.palette().primary, 0.35),
            ),
            control: blend(background, foreground, 0.16),
            tab: blend(chrome, foreground, if dark { 0.09 } else { 0.08 }),
        }
    }
    pub fn bar(self) -> container::Style {
        container::Style {
            background: Some(self.chrome.into()),
            text_color: Some(self.foreground),
            ..Default::default()
        }
    }
    pub fn panel(self) -> container::Style {
        container::Style {
            background: Some(self.menu.into()),
            text_color: Some(self.foreground),
            border: Border {
                radius: 12.0.into(),
                ..Default::default()
            },
            shadow: Shadow {
                color: Color::BLACK.scale_alpha(0.18),
                offset: Vector::new(0.0, 2.0),
                blur_radius: 8.0,
            },
            ..Default::default()
        }
    }
    pub fn tooltip(self) -> container::Style {
        let dark = self.background.r + self.background.g + self.background.b < 1.5;
        let background = if dark {
            Color::from_rgb8(16, 17, 22)
        } else {
            Color::WHITE
        };
        let foreground = if dark {
            Color::from_rgb8(245, 245, 250)
        } else {
            Color::from_rgb8(25, 25, 30)
        };
        container::Style {
            background: Some(background.into()),
            text_color: Some(foreground),
            border: Border {
                color: blend(background, foreground, 0.3),
                width: 1.0,
                radius: 6.0.into(),
            },
            shadow: Shadow {
                color: Color::BLACK.scale_alpha(0.25),
                offset: Vector::new(0.0, 2.0),
                blur_radius: 6.0,
            },
            ..Default::default()
        }
    }
    pub fn input(self, theme: &Theme, status: text_input::Status) -> text_input::Style {
        let focused = matches!(status, text_input::Status::Focused { .. });
        text_input::Style {
            background: blend(self.menu, self.foreground, 0.07).into(),
            border: Border {
                radius: 8.0.into(),
                width: if focused { 2.0 } else { 1.0 },
                color: if focused {
                    theme.palette().primary.scale_alpha(0.65)
                } else {
                    self.border
                },
            },
            icon: self.muted,
            placeholder: self.muted,
            value: self.foreground,
            selection: self.selection,
        }
    }
    pub fn flat(self, status: button::Status, selected: bool) -> button::Style {
        let background = match status {
            button::Status::Pressed => {
                Some(Background::Color(blend(self.chrome, self.foreground, 0.14)))
            }
            button::Status::Hovered => Some(Background::Color(if selected {
                blend(self.tab, self.foreground, 0.035)
            } else {
                self.tab
            })),
            _ if selected => Some(self.tab.into()),
            _ => None,
        };
        button::Style {
            background,
            text_color: if status == button::Status::Disabled {
                self.muted.scale_alpha(0.6)
            } else {
                self.foreground
            },
            border: Border {
                radius: 8.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}
