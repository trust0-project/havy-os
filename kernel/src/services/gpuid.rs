use crate::cpu::spin_delay_ms;

/// Daemon service entry point for gpuid (GPU UI daemon)
/// Handles keyboard input and GPU display updates.
/// Runs at ~60 FPS when input is detected, otherwise polls less frequently.
pub fn gpuid_service() {
    use crate::{ui, platform::d1_display, platform::d1_touch};
    use crate::services::klogd::klog_info;
    // Check if we need to transition from boot console to GUI
    if ui::boot::get_phase() == ui::boot::BootPhase::Console {
        klog_info("gpuid", "Transitioning from boot console to GUI");
        
        // Transition to GUI mode
        ui::boot::print_line("");
        ui::boot::print_boot_msg("BOOT", "System ready, starting GUI...");
        ui::boot::render();
        
        // Clear framebuffer and switch to GUI phase
        d1_display::clear_display();
        ui::boot::set_phase_gui();
        
        // Setup the boot screen UI elements
        ui::setup_main_screen();
        
        // Initial render
        ui::with_ui(|ui_mgr| {
            ui_mgr.mark_dirty();
            ui_mgr.render();
            ui_mgr.flush();
        });
        
        klog_info("gpuid", "GUI transition complete");
        return;
    }
    
    // Poll for input events
    d1_touch::poll();
    
    // Check main screen mode once before processing events (optimization)
    let is_main_screen = ui::with_ui(|ui_mgr| ui_mgr.is_main_screen_mode()).unwrap_or(false);
    
    // COALESCED event processing - drain all events, update state atomically
    // This prevents lag from intermediate mouse positions
    let mut had_input = false;
    let mut had_button_action = false;
    
    while let Some(event) = d1_touch::next_event() {
        had_input = true;
        
        if is_main_screen {
            // For mouse movement (EV_ABS), just update position - don't process fully
            // This allows coalescing of multiple movement events
            if event.event_type == d1_touch::EV_ABS {
                match event.code {
                    d1_touch::ABS_X => ui::set_cursor_pos(event.value, ui::get_cursor_pos().1),
                    d1_touch::ABS_Y => ui::set_cursor_pos(ui::get_cursor_pos().0, event.value),
                    _ => {}
                }
            } else {
                // Handle keyboard and button events immediately
                if let Some(_button) = ui::handle_main_screen_input(event) {
                    had_button_action = true;
                }
            }
        } else {
            ui::with_ui(|ui_mgr| {
                ui_mgr.handle_input(event);
            });
        }
    }
    
    // Render cursor at FINAL position (after all events processed)
    if is_main_screen {
        // No VM cursor rendering - using browser's native cursor
        // Position is updated for click hit-testing only
        
        // Only flush if there was input (button clicked) or periodically for stats
        if had_input {
            d1_display::flush();
        } else {
            // Periodically update hardware stats
            ui::update_main_screen_hardware_stats();
        }
    } else {
        ui::with_ui(|ui_mgr| {
            if ui_mgr.is_dirty() {
                ui_mgr.render();
                ui_mgr.flush();
            }
        });
    }
    
    // Return immediately - scheduler handles timing
}

/// GPU UI tick function for cooperative mode (single-hart operation)
/// Called periodically from shell_tick to handle input and render updates.
pub fn gpuid_tick() {
    use crate::{ui, platform::d1_display, platform::d1_touch};
    
    // Skip if GPU not available
    if !d1_display::is_available() {
        return;
    }
    
    // Handle boot phase transition
    if ui::boot::get_phase() == ui::boot::BootPhase::Console {
        ui::boot::print_line("");
        ui::boot::print_boot_msg("BOOT", "System ready, starting GUI...");
        ui::boot::render();
        
        // Clear and switch to GUI
        d1_display::clear_display();
        ui::boot::set_phase_gui();
        ui::setup_main_screen();
        
        ui::with_ui(|ui_mgr| {
            ui_mgr.mark_dirty();
            ui_mgr.render();
        });
        d1_display::flush();
        return;
    }
    
    // Poll for input events
    d1_touch::poll();
    
    // Check main screen mode once before processing events
    let is_main_screen = ui::with_ui(|ui_mgr| ui_mgr.is_main_screen_mode()).unwrap_or(false);
    
    // COALESCED event processing (same as gpuid_service)
    let mut had_input = false;
    while let Some(event) = d1_touch::next_event() {
        had_input = true;
        if is_main_screen {
            // For mouse movement, just update position - coalesce multiple events
            if event.event_type == d1_touch::EV_ABS {
                match event.code {
                    d1_touch::ABS_X => ui::set_cursor_pos(event.value, ui::get_cursor_pos().1),
                    d1_touch::ABS_Y => ui::set_cursor_pos(ui::get_cursor_pos().0, event.value),
                    _ => {}
                }
            } else {
                let _ = ui::handle_main_screen_input(event);
            }
        } else {
            ui::with_ui(|ui_mgr| {
                ui_mgr.handle_input(event);
            });
        }
    }
    
    // Render (no cursor - using browser's native cursor)
    if is_main_screen {
        // Position is updated for click hit-testing only
        if had_input {
            d1_display::flush();
        } else {
            ui::update_main_screen_hardware_stats();
        }
    } else {
        ui::with_ui(|ui_mgr| {
            if ui_mgr.is_dirty() {
                ui_mgr.render();
            }
        });
        d1_display::flush();
    }
}
