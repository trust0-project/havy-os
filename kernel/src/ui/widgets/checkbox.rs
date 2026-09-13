//! Checkbox Widget

use alloc::string::String;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::{Rgb888, RgbColor},
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::Text,
};

use crate::ui::colors;

/// A checkbox widget
pub struct Checkbox {
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub checked: bool,
}

impl Checkbox {
    pub fn new(label: &str, x: i32, y: i32, checked: bool) -> Self {
        Self {
            label: String::from(label),
            x,
            y,
            checked,
        }
    }

    pub fn draw<D: DrawTarget<Color = Rgb888>>(&self, target: &mut D) -> Result<(), D::Error> {
        let box_size = 14u32;

        // Checkbox background
        Rectangle::new(Point::new(self.x, self.y), Size::new(box_size, box_size))
            .into_styled(PrimitiveStyle::with_fill(colors::BUTTON_BG))
            .draw(target)?;

        // Checkbox border
        Rectangle::new(Point::new(self.x, self.y), Size::new(box_size, box_size))
            .into_styled(PrimitiveStyle::with_stroke(colors::ACCENT, 1))
            .draw(target)?;

        // Checkmark if checked
        if self.checked {
            let check_color = colors::SUCCESS;
            Line::new(
                Point::new(self.x + 3, self.y + 7),
                Point::new(self.x + 6, self.y + 11),
            )
            .into_styled(PrimitiveStyle::with_stroke(check_color, 2))
            .draw(target)?;
            Line::new(
                Point::new(self.x + 6, self.y + 11),
                Point::new(self.x + 11, self.y + 3),
            )
            .into_styled(PrimitiveStyle::with_stroke(check_color, 2))
            .draw(target)?;
        }

        // Label
        let text_style = MonoTextStyle::new(&FONT_6X10, colors::FOREGROUND);
        Text::new(
            &self.label,
            Point::new(self.x + box_size as i32 + 6, self.y + 10),
            text_style,
        )
        .draw(target)?;

        Ok(())
    }

    /// Emit HDL nodes matching [`Self::draw`] (atlas 0; v0 has no 6×10 pack).
    pub fn emit(&self, b: &mut crate::ui::scene::Builder<'_>) {
        let box_size = 14u32;
        b.panel(
            self.x,
            self.y,
            box_size,
            box_size,
            colors::BUTTON_BG.r(),
            colors::BUTTON_BG.g(),
            colors::BUTTON_BG.b(),
            0,
        );
        b.line(
            self.x,
            self.y,
            self.x + box_size as i32 - 1,
            self.y,
            colors::ACCENT.r(),
            colors::ACCENT.g(),
            colors::ACCENT.b(),
            1,
        );
        b.line(
            self.x,
            self.y + box_size as i32 - 1,
            self.x + box_size as i32 - 1,
            self.y + box_size as i32 - 1,
            colors::ACCENT.r(),
            colors::ACCENT.g(),
            colors::ACCENT.b(),
            1,
        );
        b.line(
            self.x,
            self.y,
            self.x,
            self.y + box_size as i32 - 1,
            colors::ACCENT.r(),
            colors::ACCENT.g(),
            colors::ACCENT.b(),
            1,
        );
        b.line(
            self.x + box_size as i32 - 1,
            self.y,
            self.x + box_size as i32 - 1,
            self.y + box_size as i32 - 1,
            colors::ACCENT.r(),
            colors::ACCENT.g(),
            colors::ACCENT.b(),
            1,
        );
        if self.checked {
            b.line(
                self.x + 3,
                self.y + 7,
                self.x + 6,
                self.y + 11,
                colors::SUCCESS.r(),
                colors::SUCCESS.g(),
                colors::SUCCESS.b(),
                2,
            );
            b.line(
                self.x + 6,
                self.y + 11,
                self.x + 11,
                self.y + 3,
                colors::SUCCESS.r(),
                colors::SUCCESS.g(),
                colors::SUCCESS.b(),
                2,
            );
        }
        b.label_str(
            self.x + box_size as i32 + 6,
            self.y + 10,
            colors::FOREGROUND.r(),
            colors::FOREGROUND.g(),
            colors::FOREGROUND.b(),
            crate::ui::scene::ATLAS_UI,
            &self.label,
        );
    }
}
