//! RadioButton Widget

use alloc::string::String;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::{Rgb888, RgbColor},
    prelude::*,
    primitives::{Circle, PrimitiveStyle},
    text::Text,
};

use crate::ui::colors;

/// A radio button widget
pub struct RadioButton {
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub selected: bool,
}

impl RadioButton {
    pub fn new(label: &str, x: i32, y: i32, selected: bool) -> Self {
        Self {
            label: String::from(label),
            x,
            y,
            selected,
        }
    }

    pub fn draw<D: DrawTarget<Color = Rgb888>>(&self, target: &mut D) -> Result<(), D::Error> {
        let radius = 7u32;

        // Outer circle (border)
        Circle::new(Point::new(self.x, self.y), radius * 2)
            .into_styled(PrimitiveStyle::with_stroke(colors::ACCENT, 2))
            .draw(target)?;

        // Inner circle if selected
        if self.selected {
            Circle::new(Point::new(self.x + 4, self.y + 4), (radius - 4) * 2)
                .into_styled(PrimitiveStyle::with_fill(colors::ACCENT))
                .draw(target)?;
        }

        // Label
        let text_style = MonoTextStyle::new(&FONT_6X10, colors::FOREGROUND);
        Text::new(
            &self.label,
            Point::new(self.x + (radius * 2) as i32 + 6, self.y + 10),
            text_style,
        )
        .draw(target)?;

        Ok(())
    }

    /// Emit HDL nodes. Circles are `FillRect` with radius (v0 has no circle op).
    /// `hole` is the panel behind the ring.
    pub fn emit(&self, b: &mut crate::ui::scene::Builder<'_>, hole: (u8, u8, u8)) {
        let d = 14u32;
        b.dot(
            self.x,
            self.y,
            d,
            colors::ACCENT.r(),
            colors::ACCENT.g(),
            colors::ACCENT.b(),
        );
        b.dot(self.x + 2, self.y + 2, 10, hole.0, hole.1, hole.2);
        if self.selected {
            b.dot(
                self.x + 4,
                self.y + 4,
                6,
                colors::ACCENT.r(),
                colors::ACCENT.g(),
                colors::ACCENT.b(),
            );
        }
        b.label_str(
            self.x + 20,
            self.y + 10,
            colors::FOREGROUND.r(),
            colors::FOREGROUND.g(),
            colors::FOREGROUND.b(),
            crate::ui::scene::ATLAS_UI,
            &self.label,
        );
    }
}
