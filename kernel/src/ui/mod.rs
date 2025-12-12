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

/// A window widget representing an application window
pub struct Window {
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub focused: bool,
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
        }
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

/// Last time hardware stats were updated (in ms)
static mut DEMO_LAST_HW_UPDATE: i64 = 0;

/// Hardware stats update interval in ms
const DEMO_HW_UPDATE_INTERVAL: i64 = 2000; // Update every 2 seconds

/// Update just the dynamic hardware stats section of the demo screen
/// This is much more efficient than redrawing the entire screen
pub fn update_demo_hardware_stats() {
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
    virtio_gpu::with_gpu(|gpu| {
        let col1_x = 30;
        let text_style = MonoTextStyle::new(&FONT_6X10, Rgb888::new(200, 200, 210));
        
        // Clear just the hardware stats area (lines at y=210, 225, 255, 270)
        // Each line is approximately 10 pixels high, clear a rectangle covering all
        let clear_color = Rgb888::new(28, 28, 38); // Window background color
        
        // Clear CPU line
        let _ = Rectangle::new(Point::new(col1_x, 202), Size::new(300, 12))
            .into_styled(PrimitiveStyle::with_fill(clear_color))
            .draw(gpu);
        // Clear Memory line  
        let _ = Rectangle::new(Point::new(col1_x, 217), Size::new(300, 12))
            .into_styled(PrimitiveStyle::with_fill(clear_color))
            .draw(gpu);
        // Clear Storage line
        let _ = Rectangle::new(Point::new(col1_x, 247), Size::new(300, 12))
            .into_styled(PrimitiveStyle::with_fill(clear_color))
            .draw(gpu);
        // Clear Network line
        let _ = Rectangle::new(Point::new(col1_x, 262), Size::new(300, 12))
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
    
    virtio_gpu::flush();
}

/// Fast update of just the quick action buttons (for keyboard navigation)
/// This is MUCH faster than redrawing the entire screen
pub fn update_demo_buttons(selected_button: usize) {
    virtio_gpu::with_gpu(|gpu| {
        let text_style = MonoTextStyle::new(&FONT_6X10, Rgb888::new(200, 200, 210));
        let clear_color = Rgb888::new(28, 28, 38); // Window background
        
        // Button definitions (must match draw_demo_screen_content)
        let buttons = [
            ("Terminal", 40),
            ("Network", 150),
            ("Files", 260),
            ("Settings", 370),
        ];
        
        // Clear the entire buttons area with a single rectangle (faster than 4 clears)
        let _ = Rectangle::new(Point::new(38, 338), Size::new(430, 38))
            .into_styled(PrimitiveStyle::with_fill(clear_color))
            .draw(gpu);
        
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
            
            // Button background
            let _ = RoundedRectangle::with_equal_corners(
                Rectangle::new(Point::new(*x, 340), Size::new(90, 32)),
                Size::new(4, 4),
            )
            .into_styled(PrimitiveStyle::with_fill(bg_color))
            .draw(gpu);
            
            // Button border
            let _ = RoundedRectangle::with_equal_corners(
                Rectangle::new(Point::new(*x, 340), Size::new(90, 32)),
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
            let _ = Text::new(label, Point::new(*x + 15, 360), btn_text_style).draw(gpu);
        }
    });
    
    virtio_gpu::flush();
}

/// Setup a demo screen showing embedded_graphics capabilities with dynamic hardware info
pub fn setup_demo_screen() {
    // Get hardware info
    let hw = get_hardware_info();
    
    // Reset selected button and update time
    unsafe { 
        DEMO_SELECTED_BUTTON = 0;
        DEMO_LAST_HW_UPDATE = crate::get_time_ms();
    }
    
    // Enable demo mode to prevent UI manager from overwriting our direct GPU draws
    with_ui(|ui_mgr| {
        ui_mgr.clear();
        ui_mgr.set_demo_mode(true);
    });
    
    draw_demo_screen_content(&hw, unsafe { DEMO_SELECTED_BUTTON });
}

/// Redraw the demo screen with the given selected button index
fn draw_demo_screen_content(hw: &HardwareInfo, selected_button: usize) {
    virtio_gpu::with_gpu(|gpu| {
        // Clear to dark background (desktop)
        let _ = gpu.clear(0x15, 0x15, 0x1E);
        
        // === Fullscreen Window ===
        // Window background (full width with some margin)
        let _ = Rectangle::new(Point::new(10, 10), Size::new(780, 550))
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(28, 28, 38)))
            .draw(gpu);
        
        // Window border
        let _ = Rectangle::new(Point::new(10, 10), Size::new(780, 550))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);
        
        // Title bar
        let _ = Rectangle::new(Point::new(10, 10), Size::new(780, 32))
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(40, 40, 55)))
            .draw(gpu);
        
        // Window buttons
        let _ = Circle::new(Point::new(22, 20), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(220, 80, 80)))
            .draw(gpu);
        let _ = Circle::new(Point::new(42, 20), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(230, 180, 80)))
            .draw(gpu);
        let _ = Circle::new(Point::new(62, 20), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(80, 200, 120)))
            .draw(gpu);
        
        // Title
        let title_style = MonoTextStyle::new(&FONT_9X15_BOLD, Rgb888::WHITE);
        let _ = Text::new("HAVY OS - System Information", Point::new(320, 30), title_style).draw(gpu);
        
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
        
        let _ = Text::new("Display:      800x600 VirtIO GPU", Point::new(col1_x, 240), text_style).draw(gpu);
        
        // Dynamic disk (used / total)
        let mut disk_buf = [0u8; 48];
        let disk_str = format_disk_str(hw.disk_used_kb, hw.disk_total_kb, &mut disk_buf);
        let _ = Text::new(disk_str, Point::new(col1_x, 255), text_style).draw(gpu);
        
        // Dynamic network with IP address
        let mut net_buf = [0u8; 48];
        let net_str = format_network_str(hw.network_available, &hw.ip_addr, &mut net_buf);
        let _ = Text::new(net_str, Point::new(col1_x, 270), text_style).draw(gpu);
        
        // === Right Column: Features ===
        let col2_x = 420;
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
        
        // === Quick Actions with keyboard selection ===
        let _ = Text::new("Quick Actions", Point::new(col1_x, 310), accent_style).draw(gpu);
        let _ = Line::new(Point::new(col1_x, 315), Point::new(col1_x + 120, 315))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);
        
        // Navigation hint
        let hint_style = MonoTextStyle::new(&FONT_6X10, Rgb888::new(100, 100, 120));
        let _ = Text::new("Use arrow keys to navigate, Enter to select", Point::new(col1_x, 328), hint_style).draw(gpu);
        
        // Action buttons with selection highlight
        let buttons = [
            ("Terminal", 40),
            ("Network", 150),
            ("Files", 260),
            ("Settings", 370),
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
            
            // Button background
            let _ = RoundedRectangle::with_equal_corners(
                Rectangle::new(Point::new(*x, 340), Size::new(90, 32)),
                Size::new(4, 4),
            )
            .into_styled(PrimitiveStyle::with_fill(bg_color))
            .draw(gpu);
            
            // Button border
            let _ = RoundedRectangle::with_equal_corners(
                Rectangle::new(Point::new(*x, 340), Size::new(90, 32)),
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
            let _ = Text::new(label, Point::new(*x + 15, 360), btn_text_style).draw(gpu);
        }
        
        // === Running Services (positioned to not overlap with buttons) ===
        let services_x = 500;
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
        
        // === Welcome Message (moved lower) ===
        let welcome_style = MonoTextStyle::new(&FONT_6X10, Rgb888::new(160, 160, 175));
        let _ = Text::new("HAVY OS is a lightweight operating system written in Rust, running on a", Point::new(col1_x, 400), welcome_style).draw(gpu);
        let _ = Text::new("RISC-V virtual machine in your browser.", Point::new(col1_x, 415), welcome_style).draw(gpu);
        
        // === Footer info ===
        let _ = Line::new(Point::new(30, 450), Point::new(770, 450))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);
        
        let footer_style = MonoTextStyle::new(&FONT_6X10, Rgb888::new(120, 120, 140));
        let _ = Text::new("Built with: Rust, embedded-graphics, smoltcp, wasmi", Point::new(30, 470), footer_style).draw(gpu);
        let _ = Text::new("License: MIT | github.com/elribonazo/riscv-vm", Point::new(30, 485), footer_style).draw(gpu);
        
        // Version badge
        let _ = RoundedRectangle::with_equal_corners(
            Rectangle::new(Point::new(650, 460), Size::new(120, 24)),
            Size::new(4, 4),
        )
        .into_styled(PrimitiveStyle::with_fill(Rgb888::new(80, 140, 200)))
        .draw(gpu);
        let _ = Text::new("v0.1.0-alpha", Point::new(670, 476), text_style).draw(gpu);

        // === Status Bar ===
        let _ = Rectangle::new(Point::new(0, 570), Size::new(800, 30))
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(25, 25, 35)))
            .draw(gpu);
        
        let _ = Text::new("HAVY OS | GPU Active", Point::new(10, 588), text_style).draw(gpu);
        
        // Time placeholder
        let _ = Text::new("12:00", Point::new(380, 588), text_style).draw(gpu);
        
        // Status indicators
        let net_color = if hw.network_available {
            Rgb888::new(80, 200, 120)
        } else {
            Rgb888::new(150, 150, 160)
        };
        let _ = Circle::new(Point::new(650, 577), 10)
            .into_styled(PrimitiveStyle::with_fill(net_color))
            .draw(gpu);
        let _ = Text::new("NET", Point::new(664, 588), text_style).draw(gpu);
        
        let _ = Circle::new(Point::new(700, 577), 10)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(80, 200, 120)))
            .draw(gpu);
        let _ = Text::new("CPU", Point::new(714, 588), text_style).draw(gpu);
        
        let _ = Circle::new(Point::new(750, 577), 10)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(230, 180, 80)))
            .draw(gpu);
        let _ = Text::new("MEM", Point::new(764, 588), text_style).draw(gpu);
    });
    
    virtio_gpu::flush();
}

/// Handle keyboard input for demo screen navigation
/// Returns Some(button_index) if Enter was pressed on a button
pub fn handle_demo_input(event: virtio_input::InputEvent) -> Option<usize> {
    if !event.is_key_press() {
        return None;
    }
    
    let num_buttons = 4;
    
    match event.code {
        KEY_LEFT => {
            unsafe {
                if DEMO_SELECTED_BUTTON > 0 {
                    DEMO_SELECTED_BUTTON -= 1;
                } else {
                    DEMO_SELECTED_BUTTON = num_buttons - 1;
                }
                // Use fast button-only update instead of full screen redraw
                update_demo_buttons(DEMO_SELECTED_BUTTON);
            }
            None
        }
        KEY_RIGHT => {
            unsafe {
                DEMO_SELECTED_BUTTON = (DEMO_SELECTED_BUTTON + 1) % num_buttons;
                // Use fast button-only update instead of full screen redraw
                update_demo_buttons(DEMO_SELECTED_BUTTON);
            }
            None
        }
        KEY_UP | KEY_DOWN => {
            // For a single-row button layout, up/down could also navigate
            // but for now we just refresh
            None
        }
        KEY_ENTER => {
            unsafe { Some(DEMO_SELECTED_BUTTON) }
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
