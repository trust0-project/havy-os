//! Window Widget

use alloc::string::String;
use embedded_graphics::{
    mono_font::{ascii::FONT_9X15_BOLD, MonoTextStyle},
    pixelcolor::Rgb888,
    prelude::*,
    primitives::{Circle, CornerRadii, Line, PrimitiveStyle, Rectangle, RoundedRectangle},
    text::Text,
};

use crate::ui::colors;
use crate::ui::{draw_image, LOGO_SMALL, LOGO_SMALL_SIZE};

/// A window widget representing an application window
pub struct Window {
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub focused: bool,
    /// Whether to show traffic light buttons (close, minimize, maximize)
    pub show_controls: bool,
}

impl Window {
    pub fn new(title: &str, x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            title: String::from(title),
            x,
            y,
            width,
            height,
            focused: true,
            show_controls: true, // Show controls by default
        }
    }
    
    /// Builder method to set whether controls are shown
    pub fn with_controls(mut self, show: bool) -> Self {
        self.show_controls = show;
        self
    }

    /// Draw the window to a DrawTarget
    pub fn draw<D: DrawTarget<Color = Rgb888>>(&self, target: &mut D) -> Result<(), D::Error> {
        let title_bar_height = 28u32;
        let border_color = if self.focused { colors::ACCENT } else { colors::BORDER };
        
        // Window shadow (offset dark rectangle)
        Rectangle::new(
            Point::new(self.x + 4, self.y + 4),
            Size::new(self.width, self.height),
        )
        .into_styled(PrimitiveStyle::with_fill(Rgb888::new(10, 10, 15)))
        .draw(target)?;

        // Window background
        RoundedRectangle::with_equal_corners(
            Rectangle::new(
                Point::new(self.x, self.y),
                Size::new(self.width, self.height),
            ),
            Size::new(8, 8),
        )
        .into_styled(PrimitiveStyle::with_fill(Rgb888::new(32, 32, 42)))
        .draw(target)?;

        // Title bar background
        RoundedRectangle::new(
            Rectangle::new(
                Point::new(self.x, self.y),
                Size::new(self.width, title_bar_height),
            ),
            CornerRadii {
                top_left: Size::new(8, 8),
                top_right: Size::new(8, 8),
                bottom_left: Size::zero(),
                bottom_right: Size::zero(),
            },
        )
        .into_styled(PrimitiveStyle::with_fill(Rgb888::new(45, 45, 60)))
        .draw(target)?;

        // Title bar border line
        Line::new(
            Point::new(self.x, self.y + title_bar_height as i32),
            Point::new(self.x + self.width as i32 - 1, self.y + title_bar_height as i32),
        )
        .into_styled(PrimitiveStyle::with_stroke(border_color, 1))
        .draw(target)?;

        // Window border
        RoundedRectangle::with_equal_corners(
            Rectangle::new(
                Point::new(self.x, self.y),
                Size::new(self.width, self.height),
            ),
            Size::new(8, 8),
        )
        .into_styled(PrimitiveStyle::with_stroke(border_color, 2))
        .draw(target)?;

        // Window control buttons (close, minimize, maximize)
        let button_y = self.y + 8;
        let button_radius = 6u32;
        
        // Close button (red)
        Circle::new(Point::new(self.x + 12, button_y), button_radius * 2)
            .into_styled(PrimitiveStyle::with_fill(colors::ERROR))
            .draw(target)?;
        
        // Minimize button (yellow)
        Circle::new(Point::new(self.x + 32, button_y), button_radius * 2)
            .into_styled(PrimitiveStyle::with_fill(colors::WARNING))
            .draw(target)?;
        
        // Maximize button (green)
        Circle::new(Point::new(self.x + 52, button_y), button_radius * 2)
            .into_styled(PrimitiveStyle::with_fill(colors::SUCCESS))
            .draw(target)?;

        // Window title
        let title_style = MonoTextStyle::new(&FONT_9X15_BOLD, colors::FOREGROUND);
        let title_x = self.x + 80;
        let title_y = self.y + 18;
        Text::new(&self.title, Point::new(title_x, title_y), title_style).draw(target)?;

        Ok(())
    }

    /// Get the content area rectangle (area below title bar)
    pub fn content_rect(&self) -> (i32, i32, u32, u32) {
        let title_bar_height = 28i32;
        let padding = 8i32;
        (
            self.x + padding,
            self.y + title_bar_height + padding,
            self.width - (padding * 2) as u32,
            self.height - title_bar_height as u32 - (padding * 2) as u32,
        )
    }
    
    /// Draw window with batch rendering (faster, but simpler style without rounded corners)
    /// Returns the content area for rendering content inside
    pub fn draw_fast(&self, gpu: &mut crate::d1_display::GpuDriver) -> WindowContentArea {
        const TITLE_BAR_HEIGHT: u32 = 32;
        
        // Window background - use direct fill_rect for batch rendering
        gpu.fill_rect(
            self.x as u32, 
            self.y as u32, 
            self.width, 
            self.height, 
            28, 28, 38  // Window background color
        );
        
        // Window border
        let _ = Rectangle::new(
            Point::new(self.x, self.y),
            Size::new(self.width, self.height),
        )
        .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
        .draw(gpu);
        
        // Title bar background - use direct fill_rect
        gpu.fill_rect(
            self.x as u32, 
            self.y as u32, 
            self.width, 
            TITLE_BAR_HEIGHT, 
            40, 40, 55  // Title bar color
        );
        
        // Traffic light buttons (close, minimize, maximize) - only if show_controls is true
        if self.show_controls {
            let btn_y = self.y + 10;
            let btn_start_x = self.x + 12;
            
            // Close button (red)
            let _ = Circle::new(Point::new(btn_start_x, btn_y), 12)
                .into_styled(PrimitiveStyle::with_fill(Rgb888::new(220, 80, 80)))
                .draw(gpu);
            
            // Minimize button (yellow)
            let _ = Circle::new(Point::new(btn_start_x + 20, btn_y), 12)
                .into_styled(PrimitiveStyle::with_fill(Rgb888::new(230, 180, 80)))
                .draw(gpu);
            
            // Maximize button (green)
            let _ = Circle::new(Point::new(btn_start_x + 40, btn_y), 12)
                .into_styled(PrimitiveStyle::with_fill(Rgb888::new(80, 200, 120)))
                .draw(gpu);
        }
        
        // Title text (centered)
        let title_style = MonoTextStyle::new(&FONT_9X15_BOLD, Rgb888::WHITE);
        let title_x = self.x + (self.width as i32 / 2) - ((self.title.len() as i32 * 9) / 2);
        let _ = Text::new(&self.title, Point::new(title_x, self.y + 22), title_style).draw(gpu);
        
        // Draw small logo aligned to the right of the header
        let logo_x = (self.x + self.width as i32 - LOGO_SMALL_SIZE as i32 - 8) as u32;
        let logo_y = (self.y + 4) as u32;
        draw_image(gpu, logo_x, logo_y, LOGO_SMALL_SIZE, LOGO_SMALL_SIZE, LOGO_SMALL);
        
        WindowContentArea {
            x: self.x + 1,
            y: self.y + TITLE_BAR_HEIGHT as i32 + 1,
            width: self.width - 2,
            height: self.height - TITLE_BAR_HEIGHT - 2,
        }
    }
}

/// Rectangle representing the content area inside a window
pub struct WindowContentArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
