//! UI Manager for Kernel Graphics
//!
//! Uses embedded-graphics to render UI elements (text, buttons, boxes)
//! to the VirtIO GPU framebuffer.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, ascii::FONT_9X15_BOLD, MonoTextStyle},
    pixelcolor::Rgb888,
    prelude::*,
    primitives::{
        Arc, Circle, CornerRadii, Line, PrimitiveStyle, PrimitiveStyleBuilder, 
        Rectangle, RoundedRectangle, Sector, StrokeAlignment, Triangle,
    },
    text::{Alignment, Text},
};

use crate::d1_display;
use crate::d1_touch::{self, InputEvent, KEY_DOWN, KEY_ENTER, KEY_LEFT, KEY_RIGHT, KEY_UP, 
    EV_ABS, ABS_X, ABS_Y, BTN_LEFT, BTN_RIGHT, BTN_MIDDLE, BTN_TOUCH};

// Embedded Trust0 logo (64x64 RGBA = 16KB)
static LOGO_DATA: &[u8] = include_bytes!("logo.raw");
const LOGO_WIDTH: u32 = 64;
const LOGO_HEIGHT: u32 = 64;

// Small logo for title bars (24x24 RGBA = 2KB)
static LOGO_SMALL: &[u8] = include_bytes!("logo_small.raw");
const LOGO_SMALL_SIZE: u32 = 24;

// Screen resolution constants22
pub const SCREEN_WIDTH: i32 = 1024;
pub const SCREEN_HEIGHT: i32 = 768;

/// Draw an embedded RGBA image to the framebuffer (fast blit)
fn draw_image(gpu: &mut d1_display::GpuDriver, x: u32, y: u32, width: u32, height: u32, pixels: &[u8]) {
    for row in 0..height {
        for col in 0..width {
            let i = ((row * width + col) * 4) as usize;
            if i + 3 < pixels.len() {
                let r = pixels[i];
                let g = pixels[i + 1];
                let b = pixels[i + 2];
                let a = pixels[i + 3];
                // Only draw non-transparent pixels
                if a > 128 {
                    gpu.set_pixel(x + col, y + row, r, g, b);
                }
            }
        }
    }
}

/// Mouse/cursor state
static mut CURSOR_X: i32 = 512;  // Start at center of 1024x768
static mut CURSOR_Y: i32 = 384;
static mut CURSOR_VISIBLE: bool = false;
static mut MOUSE_BUTTONS: u8 = 0;  // Bitmask: bit 0 = left, bit 1 = right, bit 2 = middle

/// Get current cursor position
pub fn get_cursor_pos() -> (i32, i32) {
    unsafe { (CURSOR_X, CURSOR_Y) }
}

/// Set cursor position (called when EV_ABS events received)
pub fn set_cursor_pos(x: i32, y: i32) {
    unsafe {
        CURSOR_X = x.clamp(0, SCREEN_WIDTH - 1);
        CURSOR_Y = y.clamp(0, SCREEN_HEIGHT - 1);
        CURSOR_VISIBLE = true;
    }
}

/// Set mouse button state
pub fn set_mouse_button(button: u16, pressed: bool) {
    unsafe {
        let bit = match button {
            BTN_LEFT => 0,
            BTN_RIGHT => 1,
            BTN_MIDDLE => 2,
            _ => return,
        };
        if pressed {
            MOUSE_BUTTONS |= 1 << bit;
        } else {
            MOUSE_BUTTONS &= !(1 << bit);
        }
    }
}

/// Get mouse button state
pub fn get_mouse_buttons() -> u8 {
    unsafe { MOUSE_BUTTONS }
}

/// Check if left mouse button is pressed
pub fn is_left_button_pressed() -> bool {
    unsafe { (MOUSE_BUTTONS & 1) != 0 }
}

/// Cursor dimensions
const CURSOR_W: usize = 12;
const CURSOR_H: usize = 16;

/// Previous cursor position for restore
static mut CURSOR_PREV_X: i32 = -100;
static mut CURSOR_PREV_Y: i32 = -100;

/// Saved pixels under cursor (12x16 = 192 pixels)
static mut CURSOR_BACKUP: [u32; CURSOR_W * CURSOR_H] = [0; CURSOR_W * CURSOR_H];
static mut CURSOR_BACKUP_VALID: bool = false;

/// Cursor bitmap (1 = white, 2 = black border, 0 = transparent)
/// Arrow cursor pointing top-left
const CURSOR_BITMAP: [u8; CURSOR_W * CURSOR_H] = [
    1,0,0,0,0,0,0,0,0,0,0,0,
    1,1,0,0,0,0,0,0,0,0,0,0,
    1,2,1,0,0,0,0,0,0,0,0,0,
    1,2,2,1,0,0,0,0,0,0,0,0,
    1,2,2,2,1,0,0,0,0,0,0,0,
    1,2,2,2,2,1,0,0,0,0,0,0,
    1,2,2,2,2,2,1,0,0,0,0,0,
    1,2,2,2,2,2,2,1,0,0,0,0,
    1,2,2,2,2,2,2,2,1,0,0,0,
    1,2,2,2,2,2,2,2,2,1,0,0,
    1,2,2,2,2,1,1,1,1,1,1,0,
    1,2,2,1,2,1,0,0,0,0,0,0,
    1,2,1,0,1,2,1,0,0,0,0,0,
    1,1,0,0,1,2,1,0,0,0,0,0,
    1,0,0,0,0,1,2,1,0,0,0,0,
    0,0,0,0,0,1,1,0,0,0,0,0,
];

/// Restore pixels under cursor (call before moving cursor)
fn restore_cursor_backup() {
    let (px, py) = unsafe { (CURSOR_PREV_X, CURSOR_PREV_Y) };
    if !unsafe { CURSOR_BACKUP_VALID } || px < 0 || py < 0 {
        return;
    }
    
    // Use batch write for faster restore
    d1_display::with_gpu(|gpu| {
        gpu.write_rect(px as u32, py as u32, CURSOR_W, CURSOR_H, 
            unsafe { &CURSOR_BACKUP }, &CURSOR_BITMAP);
    });
    
    unsafe { CURSOR_BACKUP_VALID = false; }
}

/// Save pixels under cursor location
fn save_cursor_backup(x: i32, y: i32) {
    if x < 0 || y < 0 {
        return;
    }
    
    // Use batch read for faster save
    d1_display::with_gpu(|gpu| {
        gpu.read_rect(x as u32, y as u32, CURSOR_W, CURSOR_H, 
            unsafe { &mut CURSOR_BACKUP });
    });
    unsafe { CURSOR_BACKUP_VALID = true; }
}

/// Draw cursor at current position - proper arrow pointer with bitmap
pub fn draw_cursor() {
    let (x, y) = unsafe { (CURSOR_X, CURSOR_Y) };
    let (px, py) = unsafe { (CURSOR_PREV_X, CURSOR_PREV_Y) };
    
    if !unsafe { CURSOR_VISIBLE } {
        return;
    }
    
    // Check if backup was invalidated (UI was redrawn)
    let needs_refresh = !unsafe { CURSOR_BACKUP_VALID };
    
    // Skip if position hasn't changed AND backup is valid
    if x == px && y == py && !needs_refresh {
        return;
    }
    
    // Restore previous cursor location (only if backup is valid)
    if unsafe { CURSOR_BACKUP_VALID } {
        restore_cursor_backup();
    }
    
    // Save pixels at new location
    save_cursor_backup(x, y);
    
    // Update previous position
    unsafe {
        CURSOR_PREV_X = x;
        CURSOR_PREV_Y = y;
    }
    
    // Draw cursor using batched bitmap write
    d1_display::with_gpu(|gpu| {
        gpu.draw_cursor_bitmap(x, y, CURSOR_W, CURSOR_H, &CURSOR_BITMAP);
    });
}

/// Hide cursor (restore background and mark invisible)
pub fn hide_cursor() {
    restore_cursor_backup();
    unsafe {
        CURSOR_VISIBLE = false;
        CURSOR_PREV_X = -100;
        CURSOR_PREV_Y = -100;
    }
}

/// Invalidate cursor backup (call after UI elements are redrawn to prevent ghost cursor)
/// This forces the cursor to re-save the background on next draw
pub fn invalidate_cursor_backup() {
    unsafe {
        CURSOR_BACKUP_VALID = false;
    }
}

/// Check if a point is inside a demo button, returns button index if hit
pub fn hit_test_demo_button(x: i32, y: i32) -> Option<usize> {
    // Button positions (must match draw_demo_screen_content)
    // Only Network button now, aligned left (adjusted for 1024x768)
    let buttons = [
        (60, 500, 110, 32),  // Network (left aligned)
    ];
    
    for (i, (bx, by, bw, bh)) in buttons.iter().enumerate() {
        if x >= *bx && x < bx + (*bw as i32) && y >= *by && y < by + (*bh as i32) {
            return Some(i);
        }
    }
    None
}

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
    /// When true, skip rendering (demo mode draws directly to GPU)
    demo_mode: bool,
}

impl UiManager {
    pub fn new() -> Self {
        Self {
            buttons: Vec::new(),
            labels: Vec::new(),
            selected_button: 0,
            dirty: true,
            demo_mode: false,
        }
    }
    
    /// Set demo mode - when true, render() becomes a no-op
    pub fn set_demo_mode(&mut self, mode: bool) {
        self.demo_mode = mode;
    }
    
    /// Check if in demo mode
    pub fn is_demo_mode(&self) -> bool {
        self.demo_mode
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
        // Skip rendering if in demo mode (demo draws directly to GPU)
        if self.demo_mode {
            return;
        }
        
        if !self.dirty {
            return;
        }

        d1_display::with_gpu(|gpu| {
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
        d1_display::flush();
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
    d1_display::init()?;

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
    d1_touch::poll();

    if let Some(event) = d1_touch::next_event() {
        with_ui(|ui| ui.handle_input(event)).flatten()
    } else {
        None
    }
}

/// Check if UI is initialized
pub fn is_initialized() -> bool {
    unsafe { UI_MANAGER.is_some() }
}

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
        Rectangle::new(
            Point::new(self.x, self.y),
            Size::new(box_size, box_size),
        )
        .into_styled(PrimitiveStyle::with_fill(colors::BUTTON_BG))
        .draw(target)?;

        // Checkbox border
        Rectangle::new(
            Point::new(self.x, self.y),
            Size::new(box_size, box_size),
        )
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
}

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

/// Hardware info for demo screen (fetched at runtime)
pub struct HardwareInfo {
    pub cpu_count: usize,
    pub memory_used_kb: usize,
    pub memory_total_kb: usize,
    pub disk_used_kb: usize,
    pub disk_total_kb: usize,
    pub network_available: bool,
    pub ip_addr: [u8; 4], // IP address as 4 octets
}

/// Get current hardware information from the system
pub fn get_hardware_info() -> HardwareInfo {
    use core::sync::atomic::Ordering;
    
    // Get CPU count from HARTS_ONLINE
    let cpu_count = crate::HARTS_ONLINE.load(Ordering::Relaxed);
    
    // Get memory from allocator
    let (heap_used, _heap_free) = crate::allocator::heap_stats();
    let heap_total = crate::allocator::heap_size();
    let memory_used_kb = heap_used / 1024;
    let memory_total_kb = heap_total / 1024;
    
    // Get disk usage from filesystem
    let (disk_used_kb, disk_total_kb) = {
        let fs_guard = crate::FS_STATE.read();
        if let Some(ref fs) = *fs_guard {
            let (used, total) = fs.disk_usage_bytes();
            ((used / 1024) as usize, (total / 1024) as usize)
        } else {
            (0, 0)
        }
    };
    
    // Check if network is available and get IP
    let (network_available, ip_addr) = {
        let net_guard = crate::NET_STATE.lock();
        if net_guard.is_some() {
            // Get IP from config module
            let ip = crate::net::get_my_ip();
            let octets = ip.octets();
            (true, [octets[0], octets[1], octets[2], octets[3]])
        } else {
            (false, [0, 0, 0, 0])
        }
    };
    
    HardwareInfo {
        cpu_count,
        memory_used_kb,
        memory_total_kb,
        disk_used_kb,
        disk_total_kb,
        network_available,
        ip_addr,
    }
}

/// Selected button index for keyboard navigation
static mut DEMO_SELECTED_BUTTON: usize = 0;

/// Flag to track if static content has been drawn (labels, lines, etc.)
/// When true, only dynamic content (buttons, stats) needs updating
static mut DEMO_STATIC_DRAWN: bool = false;

/// Last selected button - used to only redraw changed buttons
static mut DEMO_LAST_SELECTED: Option<usize> = None;

/// Currently open child window (None = main screen, Some(index) = button window open)
static mut DEMO_OPEN_WINDOW: Option<usize> = None;

// Window backing store - saves region behind child window for instant restore on close
// Child window: 500x400 at (260, 180), shadow: +8 pixels, total ~508x408
const WINDOW_BACKING_W: usize = 510;
const WINDOW_BACKING_H: usize = 410;
const WINDOW_BACKING_X: u32 = 258;
const WINDOW_BACKING_Y: u32 = 178;
static mut WINDOW_BACKING_STORE: [u32; WINDOW_BACKING_W * WINDOW_BACKING_H] = [0; WINDOW_BACKING_W * WINDOW_BACKING_H];
static mut WINDOW_BACKING_VALID: bool = false;

/// Save the region behind the child window before opening it
fn save_window_backing() {
    d1_display::with_gpu(|gpu| {
        unsafe {
            // FAST: Use read_rect_fast to copy entire rows at once (410 copies vs 210K reads)
            gpu.read_rect_fast(
                WINDOW_BACKING_X, WINDOW_BACKING_Y,
                WINDOW_BACKING_W, WINDOW_BACKING_H,
                &mut WINDOW_BACKING_STORE
            );
            WINDOW_BACKING_VALID = true;
        }
    });
}

/// Restore the region behind the child window (for instant close)
fn restore_window_backing() {
    if !unsafe { WINDOW_BACKING_VALID } {
        return;
    }
    d1_display::with_gpu(|gpu| {
        unsafe {
            // FAST: Use blit_rect to copy entire rows at once (410 copies vs 210K pixels)
            gpu.blit_rect(
                WINDOW_BACKING_X, WINDOW_BACKING_Y,
                WINDOW_BACKING_W, WINDOW_BACKING_H,
                &WINDOW_BACKING_STORE
            );
            WINDOW_BACKING_VALID = false;
        }
    });
}

/// Get button name for child window title
fn get_button_name(index: usize) -> &'static str {
    match index {
        0 => "Network",
        _ => "Unknown",
    }
}

/// Last time hardware stats were updated (in ms)
static mut DEMO_LAST_HW_UPDATE: i64 = 0;

/// Hardware stats update interval in ms
const DEMO_HW_UPDATE_INTERVAL: i64 = 2000; // Update every 2 seconds

/// Update just the dynamic hardware stats section of the demo screen
/// This is much more efficient than redrawing the entire screen
pub fn update_demo_hardware_stats() {
    // Don't update hardware stats if a child window is open (it would draw over the window)
    if unsafe { DEMO_OPEN_WINDOW.is_some() } {
        return;
    }
    
    let now = crate::get_time_ms();
    
    // Check if enough time has passed since last update
    let should_update = unsafe {
        if now - DEMO_LAST_HW_UPDATE < DEMO_HW_UPDATE_INTERVAL {
            return;
        }
        DEMO_LAST_HW_UPDATE = now;
        true
    };
    
    if !should_update {
        return;
    }
    
    // Get fresh hardware info
    let hw = get_hardware_info();
    
    // Only redraw the hardware stats area
    d1_display::with_gpu(|gpu| {
        let col1_x = 30;
        let text_style = MonoTextStyle::new(&FONT_6X10, Rgb888::new(200, 200, 210));
        
        // Clear just the dynamic hardware stats (NOT the static Display line at y=240)
        // Lines to clear: CPU (y=210), Memory (y=225), Disk (y=255), Network (y=270)
        // FONT_6X10 means text extends ~10px above baseline
        let clear_color = Rgb888::new(28, 28, 38); // Window background color
        
        // Clear top section: CPU (y=210) and Memory (y=225) only
        // Stop at y=228 to avoid Display line (baseline y=240, text starts ~y=230)
        let _ = Rectangle::new(Point::new(col1_x, 200), Size::new(300, 28))
            .into_styled(PrimitiveStyle::with_fill(clear_color))
            .draw(gpu);
        // Clear bottom section: Disk (y=255) and Network (y=270) only
        // Start at y=245 to avoid Display line
        let _ = Rectangle::new(Point::new(col1_x, 245), Size::new(300, 37))
            .into_styled(PrimitiveStyle::with_fill(clear_color))
            .draw(gpu);
        
        // Redraw dynamic values
        let mut cpu_buf = [0u8; 32];
        let cpu_str = format_cpu_str(hw.cpu_count, &mut cpu_buf);
        let _ = Text::new(cpu_str, Point::new(col1_x, 210), text_style).draw(gpu);
        
        let mut mem_buf = [0u8; 48];
        let mem_str = format_memory_str(hw.memory_used_kb, hw.memory_total_kb, &mut mem_buf);
        let _ = Text::new(mem_str, Point::new(col1_x, 225), text_style).draw(gpu);
        
        let mut disk_buf = [0u8; 48];
        let disk_str = format_disk_str(hw.disk_used_kb, hw.disk_total_kb, &mut disk_buf);
        let _ = Text::new(disk_str, Point::new(col1_x, 255), text_style).draw(gpu);
        
        let mut net_buf = [0u8; 48];
        let net_str = format_network_str(hw.network_available, &hw.ip_addr, &mut net_buf);
        let _ = Text::new(net_str, Point::new(col1_x, 270), text_style).draw(gpu);
    });
    
    d1_display::flush();
}

/// Fast update of just the quick action buttons (for keyboard navigation)
/// This is MUCH faster than redrawing the entire screen
pub fn update_demo_buttons(selected_button: usize) {
    // Hide cursor first (restore pixels) to prevent ghost when redrawing over it
    restore_cursor_backup();
    
    d1_display::with_gpu(|gpu| {
        let clear_color = Rgb888::new(28, 28, 38); // Window background
        
        // Button definitions - only Network now, left aligned (adjusted for 1024x768)
        let buttons = [
            ("Network", 60),
        ];
        
        // Clear the buttons area 
        // Clear the buttons area (adjusted for 1024x768)
        gpu.fill_rect(58, 498, 120, 38, 28, 28, 38);
        
        // Redraw all buttons
        for (i, (label, x)) in buttons.iter().enumerate() {
            let is_selected = i == selected_button;
            let bg_color = if is_selected {
                Rgb888::new(80, 140, 200)
            } else {
                Rgb888::new(50, 50, 70)
            };
            let border_color = if is_selected {
                Rgb888::new(120, 180, 240)
            } else {
                Rgb888::new(60, 60, 80)
            };
            
            // Button background (110 width for Network)
            let _ = RoundedRectangle::with_equal_corners(
                Rectangle::new(Point::new(*x, 500), Size::new(110, 32)),
                Size::new(4, 4),
            )
            .into_styled(PrimitiveStyle::with_fill(bg_color))
            .draw(gpu);
            
            // Button border
            let _ = RoundedRectangle::with_equal_corners(
                Rectangle::new(Point::new(*x, 500), Size::new(110, 32)),
                Size::new(4, 4),
            )
            .into_styled(PrimitiveStyle::with_stroke(border_color, if is_selected { 2 } else { 1 }))
            .draw(gpu);
            
            let text_color = if is_selected {
                Rgb888::WHITE
            } else {
                Rgb888::new(200, 200, 210)
            };
            let btn_text_style = MonoTextStyle::new(&FONT_6X10, text_color);
            let _ = Text::new(label, Point::new(*x + 25, 520), btn_text_style).draw(gpu);
        }
    });
    
    // Invalidate backup and force cursor redraw with fresh background
    invalidate_cursor_backup();
    
    d1_display::flush();
}

/// Setup a demo screen showing embedded_graphics capabilities with dynamic hardware info
pub fn setup_demo_screen() {
    // Get hardware info
    let hw = get_hardware_info();
    
    // Reset selected button and update time
    unsafe { 
        DEMO_SELECTED_BUTTON = 0;
        DEMO_STATIC_DRAWN = false;  // Force full redraw on setup
        DEMO_LAST_SELECTED = None;
        DEMO_LAST_HW_UPDATE = crate::get_time_ms();
    }
    
    // Enable demo mode to prevent UI manager from overwriting our direct GPU draws
    with_ui(|ui_mgr| {
        ui_mgr.clear();
        ui_mgr.set_demo_mode(true);
    });
    
    draw_demo_screen_content(&hw, unsafe { DEMO_SELECTED_BUTTON });
}

/// Draw a child window (opened by clicking a button)
/// Draws ONLY the child window on top of existing content for maximum speed
fn draw_child_window(_button_index: usize) {
    // PERFORMANCE: Save region behind window for instant restore on close
    save_window_backing();
    
    // Pre-compute network info BEFORE entering GPU closure (avoid locks inside)
    let net_guard = crate::NET_STATE.lock();
    let is_online = net_guard.is_some();
    drop(net_guard);
    
    let ip = crate::net::get_my_ip();
    let ip_octets = ip.octets();
    let gateway = crate::net::GATEWAY.octets();
    let dns = crate::net::DNS_SERVER.octets();
    let prefix = crate::net::PREFIX_LEN;
    
    // Pre-format strings to avoid allocations in GPU closure
    let ip_str = alloc::format!("{}.{}.{}.{}/{}", 
        ip_octets[0], ip_octets[1], ip_octets[2], ip_octets[3], prefix);
    let gw_str = alloc::format!("{}.{}.{}.{}", 
        gateway[0], gateway[1], gateway[2], gateway[3]);
    let dns_str = alloc::format!("{}.{}.{}.{}", 
        dns[0], dns[1], dns[2], dns[3]);
    
    d1_display::with_gpu(|gpu| {
        // Shadow + window background in one batch (centered for 1024x768)
        gpu.fill_rect(268, 188, 500, 400, 5, 5, 10);  // Shadow
        gpu.fill_rect(260, 180, 500, 400, 28, 28, 38);  // Window bg
        gpu.fill_rect(260, 180, 500, 32, 40, 40, 55);  // Title bar
        
        // Border (stroke only)
        let _ = Rectangle::new(Point::new(260, 180), Size::new(500, 400))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);
        
        // Traffic light buttons
        let _ = Circle::new(Point::new(272, 190), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(220, 80, 80)))
            .draw(gpu);
        let _ = Circle::new(Point::new(292, 190), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(230, 180, 80)))
            .draw(gpu);
        let _ = Circle::new(Point::new(312, 190), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(80, 200, 120)))
            .draw(gpu);
        
        // Title + logo
        let title_style = MonoTextStyle::new(&FONT_9X15_BOLD, Rgb888::WHITE);
        let _ = Text::new("Network Statistics", Point::new(430, 202), title_style).draw(gpu);
        // Small logo aligned to the right of the header (window is at x=260, width=500)
        draw_image(gpu, 260 + 500 - LOGO_SMALL_SIZE - 8, 184, LOGO_SMALL_SIZE, LOGO_SMALL_SIZE, LOGO_SMALL);
        
        // Content styles
        let label_style = MonoTextStyle::new(&FONT_6X10, Rgb888::new(230, 180, 80));
        let value_style = MonoTextStyle::new(&FONT_6X10, Rgb888::WHITE);
        let hint_style = MonoTextStyle::new(&FONT_6X10, Rgb888::new(100, 100, 120));
        
        let x = 280;
        let mut y = 240;
        
        // Device section - use static strings
        let _ = Text::new("Device:", Point::new(x, y), label_style).draw(gpu);
        y += 16;
        let _ = Text::new("Type:    VirtIO Network Device", Point::new(x + 10, y), value_style).draw(gpu);
        y += 14;
        let _ = Text::new("Address: 0x10001000", Point::new(x + 10, y), value_style).draw(gpu);
        y += 14;
        
        // Status
        if is_online {
            let _ = Text::new("Status:  * ONLINE", Point::new(x + 10, y), 
                MonoTextStyle::new(&FONT_6X10, Rgb888::new(80, 200, 120))).draw(gpu);
        } else {
            let _ = Text::new("Status:  X OFFLINE", Point::new(x + 10, y), 
                MonoTextStyle::new(&FONT_6X10, Rgb888::new(220, 80, 80))).draw(gpu);
        }
        y += 22;
        
        // Configuration
        let _ = Text::new("Configuration:", Point::new(x, y), label_style).draw(gpu);
        y += 16;
        
        // Use pre-formatted strings
        let _ = Text::new("IP:      ", Point::new(x + 10, y), value_style).draw(gpu);
        let _ = Text::new(&ip_str, Point::new(x + 64, y), value_style).draw(gpu);
        y += 14;
        let _ = Text::new("Gateway: ", Point::new(x + 10, y), value_style).draw(gpu);
        let _ = Text::new(&gw_str, Point::new(x + 64, y), value_style).draw(gpu);
        y += 14;
        let _ = Text::new("DNS:     ", Point::new(x + 10, y), value_style).draw(gpu);
        let _ = Text::new(&dns_str, Point::new(x + 64, y), value_style).draw(gpu);
        y += 22;
        
        // Protocol Stack
        let _ = Text::new("Protocol Stack:", Point::new(x, y), label_style).draw(gpu);
        y += 16;
        let _ = Text::new("smoltcp - Lightweight TCP/IP", Point::new(x + 10, y), value_style).draw(gpu);
        y += 14;
        let _ = Text::new("ICMP, UDP, TCP, ARP", Point::new(x + 10, y), value_style).draw(gpu);
        
        // Close hint
        let _ = Text::new("Press ESC or click red button to close", Point::new(330, 560), hint_style).draw(gpu);
    });
    
    // No flush here - caller will handle it
}

/// Redraw the demo screen with the given selected button index
/// Public entry point that calls inner function
fn draw_demo_screen_content(hw: &HardwareInfo, selected_button: usize) {
    draw_demo_screen_content_inner(hw, selected_button);
}

/// Inner function to draw main demo content (used by both normal draw and child window background)
fn draw_demo_screen_content_inner(hw: &HardwareInfo, selected_button: usize) {
    // Check if a child window is open - we'll draw it on top after main content
    let open_window = unsafe { DEMO_OPEN_WINDOW };
    
    // Check if static content is already drawn - skip expensive operations if so
    let static_drawn = unsafe { DEMO_STATIC_DRAWN };
    
    d1_display::with_gpu(|gpu| {
        // Only clear and draw static content if not already cached
        if !static_drawn {
            // Clear to dark background (desktop) - EXPENSIVE, skip if already drawn
            let _ = gpu.clear(0x15, 0x15, 0x1E);
        }
        
        // === Draw Window using reusable Window component (no controls) ===
        let window = Window::new("HAVY OS - System Information", 10, 10, 1004, 710)
            .with_controls(false);  // Hide traffic light buttons on main window
        let _content = window.draw_fast(gpu);
        
        // Content is positioned relative to window content area
        let text_style = MonoTextStyle::new(&FONT_6X10, Rgb888::new(200, 200, 210));
        let accent_style = MonoTextStyle::new(&FONT_6X10, Rgb888::new(80, 140, 200));
        
        // === Left Column: About ===
        let col1_x = 30;
        let _ = Text::new("About This System", Point::new(col1_x, 70), accent_style).draw(gpu);
        let _ = Line::new(Point::new(col1_x, 75), Point::new(col1_x + 150, 75))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);
        
        let _ = Text::new("OS Name:      HAVY OS", Point::new(col1_x, 95), text_style).draw(gpu);
        let _ = Text::new("Version:      0.1.0-alpha", Point::new(col1_x, 110), text_style).draw(gpu);
        let _ = Text::new("Kernel:       HavyKernel 64-bit", Point::new(col1_x, 125), text_style).draw(gpu);
        let _ = Text::new("Architecture: RISC-V RV64GC", Point::new(col1_x, 140), text_style).draw(gpu);
        let _ = Text::new("Platform:     Virtual Machine", Point::new(col1_x, 155), text_style).draw(gpu);
        
        // Hardware info section with dynamic values
        let _ = Text::new("Hardware", Point::new(col1_x, 185), accent_style).draw(gpu);
        let _ = Line::new(Point::new(col1_x, 190), Point::new(col1_x + 100, 190))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);
        
        // Dynamic CPU count
        let mut cpu_buf = [0u8; 32];
        let cpu_str = format_cpu_str(hw.cpu_count, &mut cpu_buf);
        let _ = Text::new(cpu_str, Point::new(col1_x, 210), text_style).draw(gpu);
        
        // Dynamic memory (used / total)
        let mut mem_buf = [0u8; 48];
        let mem_str = format_memory_str(hw.memory_used_kb, hw.memory_total_kb, &mut mem_buf);
        let _ = Text::new(mem_str, Point::new(col1_x, 225), text_style).draw(gpu);
        
        let _ = Text::new("Display:      1024x768 VirtIO GPU", Point::new(col1_x, 240), text_style).draw(gpu);
        
        // Dynamic disk (used / total)
        let mut disk_buf = [0u8; 48];
        let disk_str = format_disk_str(hw.disk_used_kb, hw.disk_total_kb, &mut disk_buf);
        let _ = Text::new(disk_str, Point::new(col1_x, 255), text_style).draw(gpu);
        
        // Dynamic network with IP address
        let mut net_buf = [0u8; 48];
        let net_str = format_network_str(hw.network_available, &hw.ip_addr, &mut net_buf);
        let _ = Text::new(net_str, Point::new(col1_x, 270), text_style).draw(gpu);
        
        // === Right Column: Features ===
        let col2_x = 550;
        let _ = Text::new("Features", Point::new(col2_x, 70), accent_style).draw(gpu);
        let _ = Line::new(Point::new(col2_x, 75), Point::new(col2_x + 100, 75))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);
        
        // Feature checkmarks
        let features = [
            "Multi-core SMP support",
            "Preemptive scheduler",
            "VirtIO device drivers",
            "TCP/IP networking (smoltcp)",
            "Simple File System",
            "WASM application runtime",
            "GPU-accelerated display",
            "Interactive shell",
        ];
        
        for (i, feature) in features.iter().enumerate() {
            let y = 95 + (i as i32 * 20);
            // Checkmark
            let _ = Rectangle::new(Point::new(col2_x, y - 10), Size::new(12, 12))
                .into_styled(PrimitiveStyle::with_fill(Rgb888::new(80, 200, 120)))
                .draw(gpu);
            let _ = Line::new(Point::new(col2_x + 2, y - 4), Point::new(col2_x + 5, y - 1))
                .into_styled(PrimitiveStyle::with_stroke(Rgb888::WHITE, 2))
                .draw(gpu);
            let _ = Line::new(Point::new(col2_x + 5, y - 1), Point::new(col2_x + 10, y - 8))
                .into_styled(PrimitiveStyle::with_stroke(Rgb888::WHITE, 2))
                .draw(gpu);
            let _ = Text::new(feature, Point::new(col2_x + 20, y), text_style).draw(gpu);
        }
        
        // === Quick Actions with keyboard selection (adjusted for 1024x768) ===
        let _ = Text::new("Quick Actions", Point::new(col1_x, 470), accent_style).draw(gpu);
        let _ = Line::new(Point::new(col1_x, 475), Point::new(col1_x + 120, 475))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);
        
        // Navigation hint
        let hint_style = MonoTextStyle::new(&FONT_6X10, Rgb888::new(100, 100, 120));
        let _ = Text::new("Press Enter to open Network Stats", Point::new(col1_x, 488), hint_style).draw(gpu);
        
        // Mark static content as drawn so next time we skip the expensive clear
        unsafe { DEMO_STATIC_DRAWN = true; }
        
        // Only Network button now, left aligned (adjusted for 1024x768)
        let buttons = [
            ("Network", 60),
        ];
        
        for (i, (label, x)) in buttons.iter().enumerate() {
            let is_selected = i == selected_button;
            let bg_color = if is_selected {
                Rgb888::new(80, 140, 200) // Highlight selected
            } else {
                Rgb888::new(50, 50, 70)
            };
            let border_color = if is_selected {
                Rgb888::new(120, 180, 240)
            } else {
                Rgb888::new(60, 60, 80)
            };
            
            // Button background (110 width)
            let _ = RoundedRectangle::with_equal_corners(
                Rectangle::new(Point::new(*x, 500), Size::new(110, 32)),
                Size::new(4, 4),
            )
            .into_styled(PrimitiveStyle::with_fill(bg_color))
            .draw(gpu);
            
            // Button border
            let _ = RoundedRectangle::with_equal_corners(
                Rectangle::new(Point::new(*x, 500), Size::new(110, 32)),
                Size::new(4, 4),
            )
            .into_styled(PrimitiveStyle::with_stroke(border_color, if is_selected { 2 } else { 1 }))
            .draw(gpu);
            
            let text_color = if is_selected {
                Rgb888::WHITE
            } else {
                Rgb888::new(200, 200, 210)
            };
            let btn_text_style = MonoTextStyle::new(&FONT_6X10, text_color);
            let _ = Text::new(label, Point::new(*x + 25, 520), btn_text_style).draw(gpu);
        }
        
        // === Running Services (positioned to not overlap with buttons) ===
        let services_x = 700;
        let _ = Text::new("Running Services", Point::new(services_x, 310), accent_style).draw(gpu);
        let _ = Line::new(Point::new(services_x, 315), Point::new(services_x + 140, 315))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);
        
        let services = [
            ("shell", true),
            ("httpd", true),
            ("tcpd", true),
            ("sysmond", true),
        ];
        
        // Services in a vertical list for cleaner layout
        for (i, (name, running)) in services.iter().enumerate() {
            let x = services_x;
            let y = 335 + (i as i32 * 18);
            let color = if *running { Rgb888::new(80, 200, 120) } else { Rgb888::new(150, 150, 160) };
            let _ = Circle::new(Point::new(x, y), 8)
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(gpu);
            let _ = Text::new(name, Point::new(x + 14, y + 6), text_style).draw(gpu);
        }
        
        // === Welcome Message (adjusted for 1024x768) ===
        let welcome_style = MonoTextStyle::new(&FONT_6X10, Rgb888::new(160, 160, 175));
        let _ = Text::new("HAVY OS is a lightweight operating system written in Rust, running on a", Point::new(col1_x, 560), welcome_style).draw(gpu);
        let _ = Text::new("RISC-V virtual machine in your browser.", Point::new(col1_x, 575), welcome_style).draw(gpu);
        
        // === Footer info ===
        let _ = Line::new(Point::new(30, 610), Point::new(994, 610))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);
        
        let footer_style = MonoTextStyle::new(&FONT_6X10, Rgb888::new(120, 120, 140));
        let _ = Text::new("Built with: Rust, embedded-graphics, smoltcp, wasmi", Point::new(30, 630), footer_style).draw(gpu);
        let _ = Text::new("License: MIT | github.com/elribonazo/riscv-vm", Point::new(30, 645), footer_style).draw(gpu);
        
        // Version badge
        let _ = RoundedRectangle::with_equal_corners(
            Rectangle::new(Point::new(870, 620), Size::new(120, 24)),
            Size::new(4, 4),
        )
        .into_styled(PrimitiveStyle::with_fill(Rgb888::new(80, 140, 200)))
        .draw(gpu);
        let _ = Text::new("v0.1.0-alpha", Point::new(890, 636), text_style).draw(gpu);

        // === Status Bar (at 1024x768 screen bottom) ===
        let _ = Rectangle::new(Point::new(0, 738), Size::new(1024, 30))
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(25, 25, 35)))
            .draw(gpu);
        
        let _ = Text::new("HAVY OS | GPU Active", Point::new(10, 756), text_style).draw(gpu);
        
        // Time placeholder
        let _ = Text::new("12:00", Point::new(500, 756), text_style).draw(gpu);
        
        // Status indicators
        let net_color = if hw.network_available {
            Rgb888::new(80, 200, 120)
        } else {
            Rgb888::new(150, 150, 160)
        };
        let _ = Circle::new(Point::new(870, 745), 10)
            .into_styled(PrimitiveStyle::with_fill(net_color))
            .draw(gpu);
        let _ = Text::new("NET", Point::new(884, 756), text_style).draw(gpu);
        
        let _ = Circle::new(Point::new(920, 745), 10)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(80, 200, 120)))
            .draw(gpu);
        let _ = Text::new("CPU", Point::new(934, 756), text_style).draw(gpu);
        
        let _ = Circle::new(Point::new(970, 745), 10)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(230, 180, 80)))
            .draw(gpu);
        let _ = Text::new("MEM", Point::new(984, 756), text_style).draw(gpu);
    });
    
    // If a child window is open, draw it on top of the main content
    if let Some(win_idx) = open_window {
        draw_child_window(win_idx);
    } else {
        d1_display::flush();
    }
}

/// Handle input for demo screen (keyboard navigation and mouse)
/// Returns Some(button_index) if Enter was pressed on a button
pub fn handle_demo_input(event: d1_touch::InputEvent) -> Option<usize> {
    // Check if a child window is open
    let open_window = unsafe { DEMO_OPEN_WINDOW };
    
    // Handle mouse position events
    if event.event_type == EV_ABS {
        match event.code {
            ABS_X => {
                set_cursor_pos(event.value, unsafe { CURSOR_Y });
            }
            ABS_Y => {
                set_cursor_pos(unsafe { CURSOR_X }, event.value);
            }
            _ => {}
        }
        return None;
    }
    
    // Handle mouse button events and touch events
    if event.event_type == d1_touch::EV_KEY {
        match event.code {
            BTN_LEFT | BTN_RIGHT | BTN_MIDDLE | BTN_TOUCH => {
                let pressed = event.value == 1;
                set_mouse_button(event.code, pressed);
                
                // On left mouse button or touch press
                if (event.code == BTN_LEFT || event.code == BTN_TOUCH) && pressed {
                    let (x, y) = get_cursor_pos();
                    
                    // If child window is open, check for close button click
                    if open_window.is_some() {
                        // Child window is at (260, 180, 500, 400) - centered for 1024x768
                        // Close button is at (260 + 12, 180 + 10) with radius 6
                        let close_btn_x = 260 + 12;
                        let close_btn_y = 180 + 10;
                        let dx = x - close_btn_x;
                        let dy = y - close_btn_y;
                        // Check if click is within 12px of button center (button is 12px diameter)
                        if dx * dx + dy * dy < 12 * 12 {
                            // Close the child window - use backing store for instant restore
                            unsafe { DEMO_OPEN_WINDOW = None; }
                            restore_window_backing();
                            d1_display::flush();  // Immediate update
                            return None;
                        }
                    } else {
                        // Main window - check for button clicks
                        if let Some(button_idx) = hit_test_demo_button(x, y) {
                            // Open the child window for this button
                            unsafe {
                                DEMO_SELECTED_BUTTON = button_idx;
                                DEMO_OPEN_WINDOW = Some(button_idx);
                            }
                            draw_child_window(button_idx);
                            d1_display::flush();  // Immediate update
                            return Some(button_idx);
                        }
                    }
                }
                return None;
            }
            _ => {}
        }
    }
    
    // Handle keyboard events
    if !event.is_key_press() {
        return None;
    }
    
    // If child window is open, Escape closes it
    if open_window.is_some() {
        use crate::d1_touch::KEY_ESC;
        if event.code == KEY_ESC {
            unsafe { DEMO_OPEN_WINDOW = None; }
            restore_window_backing();
            d1_display::flush();
        }
        return None;
    }
    
    let num_buttons = 1;  // Only Network button now
    
    match event.code {
        KEY_LEFT | KEY_RIGHT => {
            // Only one button, no navigation needed
            None
        }
        KEY_UP | KEY_DOWN => {
            // Only one button, no navigation needed
            None
        }
        KEY_ENTER => {
            // Open child window for selected button
            let button_idx = unsafe { DEMO_SELECTED_BUTTON };
            unsafe { DEMO_OPEN_WINDOW = Some(button_idx); }
            draw_child_window(button_idx);
            Some(button_idx)
        }
        _ => None,
    }
}

// Helper function to format CPU string
fn format_cpu_str(count: usize, buf: &mut [u8; 32]) -> &str {
    use core::fmt::Write;
    struct BufWriter<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }
    impl<'a> Write for BufWriter<'a> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let bytes = s.as_bytes();
            let remaining = self.buf.len() - self.pos;
            let to_copy = bytes.len().min(remaining);
            self.buf[self.pos..self.pos + to_copy].copy_from_slice(&bytes[..to_copy]);
            self.pos += to_copy;
            Ok(())
        }
    }
    
    let mut writer = BufWriter { buf: buf, pos: 0 };
    let _ = write!(writer, "CPU:          {} Core{} @ RISC-V", count, if count == 1 { "" } else { "s" });
    let len = writer.pos;
    core::str::from_utf8(&buf[..len]).unwrap_or("CPU: Unknown")
}

// Helper function to format memory string (used / total KB)
fn format_memory_str(used_kb: usize, total_kb: usize, buf: &mut [u8; 48]) -> &str {
    use core::fmt::Write;
    struct BufWriter<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }
    impl<'a> Write for BufWriter<'a> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let bytes = s.as_bytes();
            let remaining = self.buf.len() - self.pos;
            let to_copy = bytes.len().min(remaining);
            self.buf[self.pos..self.pos + to_copy].copy_from_slice(&bytes[..to_copy]);
            self.pos += to_copy;
            Ok(())
        }
    }
    
    let mut writer = BufWriter { buf: buf, pos: 0 };
    let _ = write!(writer, "Memory:       {}/{} KB", used_kb, total_kb);
    let len = writer.pos;
    core::str::from_utf8(&buf[..len]).unwrap_or("Memory: Unknown")
}

// Helper function to format disk string (used / total KB)
fn format_disk_str(used_kb: usize, total_kb: usize, buf: &mut [u8; 48]) -> &str {
    use core::fmt::Write;
    struct BufWriter<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }
    impl<'a> Write for BufWriter<'a> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let bytes = s.as_bytes();
            let remaining = self.buf.len() - self.pos;
            let to_copy = bytes.len().min(remaining);
            self.buf[self.pos..self.pos + to_copy].copy_from_slice(&bytes[..to_copy]);
            self.pos += to_copy;
            Ok(())
        }
    }
    
    let mut writer = BufWriter { buf: buf, pos: 0 };
    let _ = write!(writer, "Storage:      {}/{} KB", used_kb, total_kb);
    let len = writer.pos;
    core::str::from_utf8(&buf[..len]).unwrap_or("Storage: Unknown")
}

// Helper function to format network string with IP
fn format_network_str<'a>(available: bool, ip: &[u8; 4], buf: &'a mut [u8; 48]) -> &'a str {
    use core::fmt::Write;
    struct BufWriter<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }
    impl<'a> Write for BufWriter<'a> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let bytes = s.as_bytes();
            let remaining = self.buf.len() - self.pos;
            let to_copy = bytes.len().min(remaining);
            self.buf[self.pos..self.pos + to_copy].copy_from_slice(&bytes[..to_copy]);
            self.pos += to_copy;
            Ok(())
        }
    }
    
    let mut writer = BufWriter { buf: buf, pos: 0 };
    if available {
        let _ = write!(writer, "Network:      {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
    } else {
        let _ = write!(writer, "Network:      Not connected");
    }
    let len = writer.pos;
    core::str::from_utf8(&buf[..len]).unwrap_or("Network: Unknown")
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
