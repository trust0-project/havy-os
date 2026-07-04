use core::sync::atomic::{AtomicBool, Ordering};
use crate::cpu::spin_delay_ms;

/// Flag set by the interrupt handler when input events are ready.
/// gpuid checks this to know when to wake up and process input.
static INPUT_READY: AtomicBool = AtomicBool::new(false);

/// Signal that input events are ready to be processed.
///
/// Called from the external interrupt handler when PLIC delivers
/// a D1 Touch or VirtIO Input interrupt. This wakes up gpuid
/// to process the pending input events.
pub fn signal_input_ready() {
    INPUT_READY.store(true, Ordering::Release);
}

/// Check if input events are pending.
#[inline]
pub fn is_input_ready() -> bool {
    INPUT_READY.load(Ordering::Acquire)
}

/// Clear the input ready flag after processing events.
#[inline]
fn clear_input_ready() {
    INPUT_READY.store(false, Ordering::Release);
}

/// Timestamp (ms) of the last hardware-stats refresh.
static LAST_STATS_MS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Hardware-stats refresh interval (ms). Matches the old 2 s cadence.
const STATS_REFRESH_MS: u64 = 2000;

/// Whether the periodic hardware-stats refresh is due (and record it).
fn stats_refresh_due() -> bool {
    let now = crate::get_time_ms() as u64;
    let last = LAST_STATS_MS.load(Ordering::Relaxed);
    if now.wrapping_sub(last) >= STATS_REFRESH_MS {
        LAST_STATS_MS.store(now, Ordering::Relaxed);
        true
    } else {
        false
    }
}

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
    
    crate::perfstat::inc(crate::perfstat::id::GPUID_TICKS);
    
    // Event-driven gate: once the GUI is up, skip the full poll/render path
    // unless there is input to process, the framebuffer is dirty, or the
    // periodic hardware-stats refresh is due. The PLIC input handler sets
    // INPUT_READY; without this the daemon re-ran its whole path every tick.
    if ui::boot::get_phase() != ui::boot::BootPhase::Console {
        let due_for_stats = stats_refresh_due();
        if !is_input_ready()
            && !crate::platform::d1_display::is_frame_dirty()
            && !due_for_stats
        {
            // Nothing to do: park briefly so the hart can idle.
            crate::cpu::sched::sleep_current_ms(8);
            return;
        }
    }
    
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
    
    // Consume the input-ready signal: we're about to drain all pending
    // events, so clear it now (new events set it again via the PLIC handler).
    clear_input_ready();

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
    let flushed = crate::platform::d1_display::is_frame_dirty();
    if flushed {
        display_proxy::flush();
    }
    if had_input || had_button_action || flushed {
        crate::perfstat::inc(crate::perfstat::id::GPUID_ACTIVE_TICKS);
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
