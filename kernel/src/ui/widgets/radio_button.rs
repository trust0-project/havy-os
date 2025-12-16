//! RadioButton Widget

use alloc::string::String;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::Rgb888,
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
}
