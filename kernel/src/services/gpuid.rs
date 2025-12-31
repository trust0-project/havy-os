use crate::cpu::spin_delay_ms;

/// Daemon service entry point for gpuid (GPU UI daemon)
/// Handles keyboard input and GPU display updates.
/// Runs at ~60 FPS when input is detected, otherwise polls less frequently.
/// 
/// Multi-hart safe: Uses display_proxy to delegate hardware access to Hart 0.
pub fn gpuid_service() {
    use crate::ui;
    use crate::cpu::display_proxy;
    use crate::platform::d1_touch::{EV_ABS, ABS_X, ABS_Y}; // Constants only
    use crate::services::klogd::klog_info;
    
    // Check if we need to transition from boot console to GUI
    if ui::boot::get_phase() == ui::boot::BootPhase::Console {
        klog_info("gpuid", "Transitioning from boot console to GUI");
        
        // Transition to GUI mode
        ui::boot::print_line("");
        ui::boot::print_boot_msg("BOOT", "System ready, starting GUI...");
        ui::boot::render();
        
        // Clear framebuffer and switch to GUI phase
        display_proxy::clear_display();
        ui::boot::set_phase_gui();
        
        // Setup the boot screen UI elements
        ui::setup_main_screen();
        
        // Initial render (flush will happen at end of function)
        ui::with_ui(|ui_mgr| {
            ui_mgr.mark_dirty();
            ui_mgr.render();
        });
        
        // Deferred flush at end of frame
        if crate::platform::d1_display::is_frame_dirty() {
            display_proxy::flush();
        }
        
        klog_info("gpuid", "GUI transition complete");
        return;
    }
    
    // Poll for input events (proxied to Hart 0 if needed)
    display_proxy::touch_poll();
    
    // Check main screen mode once before processing events (optimization)
    let is_main_screen = ui::with_ui(|ui_mgr| ui_mgr.is_main_screen_mode()).unwrap_or(false);
    
    // COALESCED event processing - drain all events, update state atomically
    // This prevents lag from intermediate mouse positions
    let mut had_input = false;
    let mut had_button_action = false;
    
    while let Some(event) = display_proxy::touch_next_event() {
        had_input = true;
        
        if is_main_screen {
            // For mouse movement (EV_ABS), just update position - don't process fully
            // This allows coalescing of multiple movement events
            if event.event_type == EV_ABS {
                match event.code {
                    ABS_X => ui::set_cursor_pos(event.value, ui::get_cursor_pos().1),
                    ABS_Y => ui::set_cursor_pos(ui::get_cursor_pos().0, event.value),
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
        
        // Check for GUI command completion (async polling)
        ui::main_screen::check_gui_command_completion();
        
        // Periodically update hardware stats (no input needed)
        if !had_input {
            ui::update_main_screen_hardware_stats();
        }
    } else {
        ui::with_ui(|ui_mgr| {
            if ui_mgr.is_dirty() {
                ui_mgr.render();
            }
        });
    }
    
    // Deferred flush: single flush at end of frame if anything was drawn
    if crate::platform::d1_display::is_frame_dirty() {
        display_proxy::flush();
    }
    
    // Return immediately - scheduler handles timing
}

/// GPU UI tick function for cooperative mode (single-hart operation)
/// Called periodically from shell_tick to handle input and render updates.
/// 
/// Multi-hart safe: Uses display_proxy to delegate hardware access to Hart 0.
pub fn gpuid_tick() {
    use crate::ui;
    use crate::cpu::display_proxy;
    use crate::platform::d1_touch::{EV_ABS, ABS_X, ABS_Y}; // Constants only
    
    // Skip if GPU not available (proxied check)
    if !display_proxy::is_available() {
        return;
    }
    
    // Handle boot phase transition
    if ui::boot::get_phase() == ui::boot::BootPhase::Console {
        ui::boot::print_line("");
        ui::boot::print_boot_msg("BOOT", "System ready, starting GUI...");
        ui::boot::render();
        
        // Clear and switch to GUI
        display_proxy::clear_display();
        ui::boot::set_phase_gui();
        ui::setup_main_screen();
        
        ui::with_ui(|ui_mgr| {
            ui_mgr.mark_dirty();
            ui_mgr.render();
        });
        // Deferred flush at end of frame
        if crate::platform::d1_display::is_frame_dirty() {
            display_proxy::flush();
        }
        return;
    }
    
    // Poll for input events (proxied to Hart 0 if needed)
    display_proxy::touch_poll();
    
    // Check main screen mode once before processing events
    let is_main_screen = ui::with_ui(|ui_mgr| ui_mgr.is_main_screen_mode()).unwrap_or(false);
    
    // COALESCED event processing (same as gpuid_service)
    let mut had_input = false;
    while let Some(event) = display_proxy::touch_next_event() {
        had_input = true;
        if is_main_screen {
            // For mouse movement, just update position - coalesce multiple events
            if event.event_type == EV_ABS {
                match event.code {
                    ABS_X => ui::set_cursor_pos(event.value, ui::get_cursor_pos().1),
                    ABS_Y => ui::set_cursor_pos(ui::get_cursor_pos().0, event.value),
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
        // Check for GUI command completion (async polling)
        ui::main_screen::check_gui_command_completion();
        
        // Periodically update hardware stats (no input needed)
        if !had_input {
            ui::update_main_screen_hardware_stats();
        }
    } else {
        ui::with_ui(|ui_mgr| {
            if ui_mgr.is_dirty() {
                ui_mgr.render();
            }
        });
    }
    
    // Deferred flush: single flush at end of frame if anything was drawn
    if crate::platform::d1_display::is_frame_dirty() {
        display_proxy::flush();
    }
}
