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
pub static mut UI_MANAGER: Option<UiManager> = None;

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
/// Displays the same boot messages as UART output from main.rs
pub fn setup_boot_screen() {
    with_ui(|ui_mgr| {
        let mut y = 20;
        let line_height = 14;
        let x_margin = 20;
        let x_indent = 40;
        
        // Header
        ui_mgr.add_label(Label::new("HAVY OS Boot", x_margin, y).with_color(colors::ACCENT));
        y += line_height + 4;
        
        // --- CPU & ARCHITECTURE ---
        ui_mgr.add_label(Label::new("* CPU & ARCHITECTURE", x_margin, y).with_color(colors::WARNING));
        y += line_height;
        ui_mgr.add_label(Label::new("  +- Primary Hart: 0", x_indent, y));
        y += line_height;
        ui_mgr.add_label(Label::new("  +- Architecture: RISC-V 64-bit (RV64GC)", x_indent, y));
        y += line_height;
        ui_mgr.add_label(Label::new("  +- Mode: Machine Mode (M-Mode)", x_indent, y));
        y += line_height;
        ui_mgr.add_label(Label::new("  +- Timer Source: CLINT @ 0x02000000", x_indent, y));
        y += line_height;
        ui_mgr.add_label(Label::new("[OK] CPU initialized", x_indent, y).with_color(colors::SUCCESS));
        y += line_height + 4;
        
        // --- MEMORY SUBSYSTEM ---
        ui_mgr.add_label(Label::new("* MEMORY SUBSYSTEM", x_margin, y).with_color(colors::WARNING));
        y += line_height;
        ui_mgr.add_label(Label::new("  +- Heap Base: 0x80800000", x_indent, y));
        y += line_height;
        ui_mgr.add_label(Label::new("  +- Heap Size: 8192 KiB", x_indent, y));
        y += line_height;
        ui_mgr.add_label(Label::new("[OK] Heap allocator ready", x_indent, y).with_color(colors::SUCCESS));
        y += line_height + 4;
        
        // --- STORAGE SUBSYSTEM ---
        ui_mgr.add_label(Label::new("* STORAGE SUBSYSTEM", x_margin, y).with_color(colors::WARNING));
        y += line_height;
        ui_mgr.add_label(Label::new("[OK] VirtIO Block device initialized", x_indent, y).with_color(colors::SUCCESS));
        y += line_height;
        ui_mgr.add_label(Label::new("[OK] Filesystem mounted", x_indent, y).with_color(colors::SUCCESS));
        y += line_height + 4;
        
        // --- NETWORK SUBSYSTEM ---
        ui_mgr.add_label(Label::new("* NETWORK SUBSYSTEM", x_margin, y).with_color(colors::WARNING));
        y += line_height;
        ui_mgr.add_label(Label::new("[OK] VirtIO Net initialized", x_indent, y).with_color(colors::SUCCESS));
        y += line_height;
        ui_mgr.add_label(Label::new("  +- Acquiring IP via DHCP...", x_indent, y));
        y += line_height + 4;
        
        // --- GPU SUBSYSTEM ---
        ui_mgr.add_label(Label::new("* GPU SUBSYSTEM", x_margin, y).with_color(colors::WARNING));
        y += line_height;
        ui_mgr.add_label(Label::new("[OK] VirtIO GPU initialized", x_indent, y).with_color(colors::SUCCESS));
        y += line_height;
        ui_mgr.add_label(Label::new("[OK] UI Manager initialized", x_indent, y).with_color(colors::SUCCESS));
        y += line_height;
        ui_mgr.add_label(Label::new("  +- Display: 800x600 @ 32bpp", x_indent, y));
        y += line_height + 4;
        
        // --- SMP INITIALIZATION ---
        ui_mgr.add_label(Label::new("* SMP INITIALIZATION", x_margin, y).with_color(colors::WARNING));
        y += line_height;
        ui_mgr.add_label(Label::new("[OK] All harts online", x_indent, y).with_color(colors::SUCCESS));
        y += line_height + 4;
        
        // --- PROCESS MANAGER ---
        ui_mgr.add_label(Label::new("* PROCESS MANAGER", x_margin, y).with_color(colors::WARNING));
        y += line_height;
        ui_mgr.add_label(Label::new("[OK] CPU table initialized", x_indent, y).with_color(colors::SUCCESS));
        y += line_height;
        ui_mgr.add_label(Label::new("[OK] Process scheduler initialized", x_indent, y).with_color(colors::SUCCESS));
        y += line_height;
        ui_mgr.add_label(Label::new("[OK] System services started", x_indent, y).with_color(colors::SUCCESS));
        y += line_height + 8;
        
        // Status line
        ui_mgr.add_label(Label::new("System Running - Press keys to interact", x_margin, y).with_color(colors::ACCENT));
        y += line_height + 16;
        
        // Add navigation buttons at the bottom
        let button_y = 540;
        let buttons = [
            Button::new("Terminal", 80, button_y, 120, 35),
            Button::new("System Info", 220, button_y, 120, 35),
            Button::new("Network", 360, button_y, 120, 35),
            Button::new("Help", 500, button_y, 120, 35),
        ];
        
        for (i, mut button) in buttons.into_iter().enumerate() {
            if i == 0 {
                button.selected = true;
            }
            ui_mgr.add_button(button);
        }
        
        ui_mgr.mark_dirty();
    });
}
