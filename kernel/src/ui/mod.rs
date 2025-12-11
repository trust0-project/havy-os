//! UI Manager for Kernel Graphics
//!
//! Uses embedded-graphics to render UI elements (text, buttons, boxes)
//! to the VirtIO GPU framebuffer.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::Rgb888,
    prelude::*,
    primitives::{Circle, Line, PrimitiveStyle, Rectangle, RoundedRectangle},
    text::{Alignment, Text},
};

use crate::virtio_gpu;
use crate::virtio_input::{self, InputEvent, KEY_DOWN, KEY_ENTER, KEY_LEFT, KEY_RIGHT, KEY_UP};

/// UI Theme colors
pub mod colors {
    use embedded_graphics::pixelcolor::Rgb888;

    pub const BACKGROUND: Rgb888 = Rgb888::new(24, 24, 32);
    pub const FOREGROUND: Rgb888 = Rgb888::new(220, 220, 230);
    pub const ACCENT: Rgb888 = Rgb888::new(80, 140, 200);
    pub const ACCENT_HIGHLIGHT: Rgb888 = Rgb888::new(100, 160, 220);
    pub const SUCCESS: Rgb888 = Rgb888::new(80, 200, 120);
    pub const WARNING: Rgb888 = Rgb888::new(230, 180, 80);
    pub const ERROR: Rgb888 = Rgb888::new(220, 80, 80);
    pub const BORDER: Rgb888 = Rgb888::new(60, 60, 80);
    pub const BUTTON_BG: Rgb888 = Rgb888::new(50, 50, 70);
    pub const BUTTON_SELECTED: Rgb888 = Rgb888::new(80, 140, 200);
}

/// A simple button widget
#[derive(Clone)]
pub struct Button {
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub selected: bool,
}

impl Button {
    pub fn new(label: &str, x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            label: String::from(label),
            x,
            y,
            width,
            height,
            selected: false,
        }
    }

    /// Draw the button to a DrawTarget
    pub fn draw<D: DrawTarget<Color = Rgb888>>(&self, target: &mut D) -> Result<(), D::Error> {
        let bg_color = if self.selected {
            colors::BUTTON_SELECTED
        } else {
            colors::BUTTON_BG
        };

        // Draw rounded rectangle background
        let rect = RoundedRectangle::with_equal_corners(
            Rectangle::new(
                Point::new(self.x, self.y),
                Size::new(self.width, self.height),
            ),
            Size::new(4, 4),
        );
        rect.into_styled(PrimitiveStyle::with_fill(bg_color))
            .draw(target)?;

        // Draw border
        rect.into_styled(PrimitiveStyle::with_stroke(colors::BORDER, 1))
            .draw(target)?;

        // Draw label centered
        let text_style = MonoTextStyle::new(&FONT_6X10, colors::FOREGROUND);
        let center_x = self.x + (self.width as i32 / 2);
        let center_y = self.y + (self.height as i32 / 2) + 3; // +3 for font baseline

        Text::with_alignment(
            &self.label,
            Point::new(center_x, center_y),
            text_style,
            Alignment::Center,
        )
        .draw(target)?;

        Ok(())
    }
}

/// A text label widget
pub struct Label {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub color: Rgb888,
}

impl Label {
    pub fn new(text: &str, x: i32, y: i32) -> Self {
        Self {
            text: String::from(text),
            x,
            y,
            color: colors::FOREGROUND,
        }
    }

    pub fn with_color(mut self, color: Rgb888) -> Self {
        self.color = color;
        self
    }

    pub fn draw<D: DrawTarget<Color = Rgb888>>(&self, target: &mut D) -> Result<(), D::Error> {
        let text_style = MonoTextStyle::new(&FONT_6X10, self.color);
        Text::new(&self.text, Point::new(self.x, self.y), text_style).draw(target)?;
        Ok(())
    }
}

/// A progress bar widget
pub struct ProgressBar {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub progress: f32, // 0.0 to 1.0
    pub color: Rgb888,
}

impl ProgressBar {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            progress: 0.0,
            color: colors::ACCENT,
        }
    }

    pub fn set_progress(&mut self, progress: f32) {
        self.progress = progress.clamp(0.0, 1.0);
    }

    pub fn draw<D: DrawTarget<Color = Rgb888>>(&self, target: &mut D) -> Result<(), D::Error> {
        // Background
        Rectangle::new(
            Point::new(self.x, self.y),
            Size::new(self.width, self.height),
        )
        .into_styled(PrimitiveStyle::with_fill(colors::BUTTON_BG))
        .draw(target)?;

        // Fill
        let fill_width = ((self.width as f32 * self.progress) as u32).max(1);
        Rectangle::new(
            Point::new(self.x, self.y),
            Size::new(fill_width, self.height),
        )
        .into_styled(PrimitiveStyle::with_fill(self.color))
        .draw(target)?;

        // Border
        Rectangle::new(
            Point::new(self.x, self.y),
            Size::new(self.width, self.height),
        )
        .into_styled(PrimitiveStyle::with_stroke(colors::BORDER, 1))
        .draw(target)?;

        Ok(())
    }
}

/// A simple box/panel widget
pub struct Panel {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub title: Option<String>,
}

impl Panel {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            title: None,
        }
    }

    pub fn with_title(mut self, title: &str) -> Self {
        self.title = Some(String::from(title));
        self
    }

    pub fn draw<D: DrawTarget<Color = Rgb888>>(&self, target: &mut D) -> Result<(), D::Error> {
        // Background
        Rectangle::new(
            Point::new(self.x, self.y),
            Size::new(self.width, self.height),
        )
        .into_styled(PrimitiveStyle::with_fill(colors::BACKGROUND))
        .draw(target)?;

        // Border
        Rectangle::new(
            Point::new(self.x, self.y),
            Size::new(self.width, self.height),
        )
        .into_styled(PrimitiveStyle::with_stroke(colors::BORDER, 1))
        .draw(target)?;

        // Title if present
        if let Some(ref title) = self.title {
            let text_style = MonoTextStyle::new(&FONT_6X10, colors::ACCENT);
            Text::new(title, Point::new(self.x + 8, self.y + 14), text_style).draw(target)?;

            // Title underline
            Line::new(
                Point::new(self.x + 4, self.y + 18),
                Point::new(self.x + self.width as i32 - 4, self.y + 18),
            )
            .into_styled(PrimitiveStyle::with_stroke(colors::BORDER, 1))
            .draw(target)?;
        }

        Ok(())
    }
}

/// UI Manager state
pub struct UiManager {
    buttons: Vec<Button>,
    labels: Vec<Label>,
    selected_button: usize,
    dirty: bool,
}

impl UiManager {
    pub fn new() -> Self {
        Self {
            buttons: Vec::new(),
            labels: Vec::new(),
            selected_button: 0,
            dirty: true,
        }
    }

    /// Add a button to the UI
    pub fn add_button(&mut self, button: Button) -> usize {
        let idx = self.buttons.len();
        self.buttons.push(button);
        if idx == 0 {
            self.buttons[0].selected = true;
        }
        self.dirty = true;
        idx
    }

    /// Add a label to the UI
    pub fn add_label(&mut self, label: Label) {
        self.labels.push(label);
        self.dirty = true;
    }

    /// Handle an input event
    pub fn handle_input(&mut self, event: InputEvent) -> Option<usize> {
        if !event.is_key_press() {
            return None;
        }

        match event.code {
            KEY_UP | KEY_LEFT => {
                self.select_previous();
                None
            }
            KEY_DOWN | KEY_RIGHT => {
                self.select_next();
                None
            }
            KEY_ENTER => {
                // Return the index of the selected button
                if !self.buttons.is_empty() {
                    Some(self.selected_button)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Select the next button
    pub fn select_next(&mut self) {
        if self.buttons.is_empty() {
            return;
        }
        self.buttons[self.selected_button].selected = false;
        self.selected_button = (self.selected_button + 1) % self.buttons.len();
        self.buttons[self.selected_button].selected = true;
        self.dirty = true;
    }

    /// Select the previous button
    pub fn select_previous(&mut self) {
        if self.buttons.is_empty() {
            return;
        }
        self.buttons[self.selected_button].selected = false;
        self.selected_button = if self.selected_button == 0 {
            self.buttons.len() - 1
        } else {
            self.selected_button - 1
        };
        self.buttons[self.selected_button].selected = true;
        self.dirty = true;
    }

    /// Render the UI to the GPU framebuffer
    pub fn render(&mut self) {
        if !self.dirty {
            return;
        }

        virtio_gpu::with_gpu(|gpu| {
            // Clear background
            let _ = gpu.clear(
                colors::BACKGROUND.r(),
                colors::BACKGROUND.g(),
                colors::BACKGROUND.b(),
            );

            // Draw all labels
            for label in &self.labels {
                let _ = label.draw(gpu);
            }

            // Draw all buttons
            for button in &self.buttons {
                let _ = button.draw(gpu);
            }
        });

        self.dirty = false;
    }

    /// Flush the framebuffer to display
    pub fn flush(&self) {
        virtio_gpu::flush();
    }

    /// Check if UI needs redraw
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Mark UI as needing redraw
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Clear all widgets
    pub fn clear(&mut self) {
        self.buttons.clear();
        self.labels.clear();
        self.selected_button = 0;
        self.dirty = true;
    }
}

/// Global UI manager instance
static mut UI_MANAGER: Option<UiManager> = None;

/// Initialize the UI manager
pub fn init() -> Result<(), &'static str> {
    // First initialize the GPU
    virtio_gpu::init()?;

    unsafe {
        UI_MANAGER = Some(UiManager::new());
    }

    Ok(())
}

/// Get access to the UI manager
pub fn with_ui<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut UiManager) -> R,
{
    unsafe { UI_MANAGER.as_mut().map(f) }
}

/// Render and flush the UI
pub fn render_and_flush() {
    with_ui(|ui| {
        ui.render();
        ui.flush();
    });
}

/// Poll for input and handle it
pub fn poll_input() -> Option<usize> {
    virtio_input::poll();

    if let Some(event) = virtio_input::next_event() {
        with_ui(|ui| ui.handle_input(event)).flatten()
    } else {
        None
    }
}

/// Check if UI is initialized
pub fn is_initialized() -> bool {
    unsafe { UI_MANAGER.is_some() }
}

/// Setup the boot screen UI elements
/// This populates the UI with the boot screen widgets without rendering.
/// Call render_and_flush() to actually display them.
pub fn setup_boot_screen() {
    with_ui(|ui_mgr| {
        // Add system info labels
        let labels = [
            Label::new("Welcome to HAVY OS", 80, 100).with_color(colors::ACCENT),
            Label::new("A bare-metal operating system for RISC-V", 80, 120),
            Label::new("", 80, 150),
            Label::new("System Information:", 80, 170).with_color(colors::SUCCESS),
            Label::new("  Architecture: RISC-V 64-bit (RV64GC)", 80, 190),
            Label::new("  Display: 800x600 @ 32bpp", 80, 210),
            Label::new("  Status: Running", 80, 230),
            Label::new("", 80, 260),
            Label::new("Use arrow keys to navigate, Enter to select.", 80, 280).with_color(colors::WARNING),
            Label::new("Toggle display mode to switch views.", 80, 300),
        ];
        
        for label in labels {
            ui_mgr.add_label(label);
        }
        
        // Add navigation buttons
        let buttons = [
            Button::new("Terminal", 80, 400, 120, 35),
            Button::new("System Info", 220, 400, 120, 35),
            Button::new("Network", 360, 400, 120, 35),
            Button::new("Help", 500, 400, 120, 35),
        ];
        
        for (i, mut button) in buttons.into_iter().enumerate() {
            if i == 0 {
                button.selected = true; // First button selected by default
            }
            ui_mgr.add_button(button);
        }
        
        ui_mgr.mark_dirty();
    });
}
