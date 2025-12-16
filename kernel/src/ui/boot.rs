//! Boot Screen
//!
//! Setup and display of the boot screen UI.

use crate::ui::colors;
use crate::ui::manager::with_ui;
use crate::ui::widgets::{Button, Label};

/// Setup the boot screen UI elements
/// This populates the UI with the boot screen widgets without rendering.
/// Call render_and_flush() to actually display them.
/// Displays the same boot messages as UART output from main.rs
pub fn setup_boot_screen() {
    with_ui(|ui_mgr| {
        let mut y = 20;
        let line_height = 18;
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
