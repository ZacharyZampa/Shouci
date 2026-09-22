//! Semantic style slots for the TUI.
//!
//! Widget code must use these slots instead of hardcoding palette values.
//! [`Theme::detect`] honors `NO_COLOR`: any non-empty value selects the
//! monochrome theme (text modifiers only, no hues), so status stays readable
//! when color is disabled.

use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Theme {
    monochrome: bool,
}

impl Theme {
    /// Detect the theme from the environment. A non-empty `NO_COLOR`
    /// selects monochrome; everything else gets the color theme.
    #[must_use]
    pub(crate) fn detect() -> Self {
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        Self {
            monochrome: no_color,
        }
    }

    /// Monochrome theme: modifiers only, no hues. Always safe.
    /// Test helper; production uses [`Theme::detect`].
    #[cfg(test)]
    #[must_use]
    pub(crate) fn monochrome() -> Self {
        Self { monochrome: true }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn is_monochrome(self) -> bool {
        self.monochrome
    }

    fn hue(self, color: Color) -> Option<Color> {
        if self.monochrome { None } else { Some(color) }
    }

    fn slot(self, color: Color, modifier: Modifier) -> Style {
        let mut style = Style::default().add_modifier(modifier);
        if let Some(color) = self.hue(color) {
            style = style.fg(color);
        }
        style
    }

    fn plain_hue(self, color: Color) -> Style {
        self.hue(color)
            .map_or_else(Style::default, |c| Style::default().fg(c))
    }

    /// Panel titles and the `shouci` brand: bold, cyan when colored.
    #[must_use]
    pub(crate) fn accent(self) -> Style {
        self.slot(Color::Cyan, Modifier::BOLD)
    }

    /// Help-footer key hints (`F1`, `Enter`): bold, yellow when colored.
    #[must_use]
    pub(crate) fn key(self) -> Style {
        self.slot(Color::Yellow, Modifier::BOLD)
    }

    /// Secondary text (glosses, separators, counts): dimmed.
    #[must_use]
    pub(crate) fn dim() -> Style {
        Style::default().add_modifier(Modifier::DIM)
    }

    /// Selected list row. `REVERSED` survives monochrome by construction.
    #[must_use]
    pub(crate) fn selected() -> Style {
        Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    }

    /// Border of the focused panel: cyan when colored, default otherwise.
    #[must_use]
    pub(crate) fn focus_border(self) -> Style {
        self.plain_hue(Color::Cyan)
    }

    /// Border of inactive panels.
    #[must_use]
    pub(crate) fn plain_border() -> Style {
        Style::default()
    }

    /// Durable errors: red + bold when colored, bold when monochrome.
    /// Always paired with literal text, never color alone.
    #[must_use]
    pub(crate) fn error(self) -> Style {
        self.slot(Color::Red, Modifier::BOLD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monochrome_theme_uses_no_hues() {
        let theme = Theme::monochrome();
        assert!(theme.is_monochrome());
        for style in [
            theme.accent(),
            theme.key(),
            theme.focus_border(),
            Theme::plain_border(),
            theme.error(),
        ] {
            assert_eq!(style.fg, None, "monochrome slot must not set fg");
        }
    }

    #[test]
    fn selection_survives_without_color() {
        // REVERSED + BOLD is visible on any terminal, with or without hues.
        let style = Theme::selected();
        assert!(style.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn error_is_bold_even_monochrome() {
        let style = Theme::monochrome().error();
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }
}
