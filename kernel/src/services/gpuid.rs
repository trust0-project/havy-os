use crate::cpu::spin_delay_ms;
use core::sync::atomic::{AtomicBool, Ordering};

/// Flag set by the interrupt handler when input events are ready.
/// gpuid checks this to know when to wake up and process input.
static INPUT_READY: AtomicBool = AtomicBool::new(false);

/// Signal that input events are ready to be processed.
///
/// Called from the external interrupt handler when PLIC delivers
/// a D1 Touch or VirtIO Input interrupt. This wakes gpuid immediately
/// so a click is not stuck behind `sleep_current_ms`.
pub fn signal_input_ready() {
    INPUT_READY.store(true, Ordering::Release);
    crate::cpu::sched::handle_ipi(crate::get_hart_id());
}

/// Parked `gpuid` is runnable even before its sleep deadline when HID or
/// queued UI work (WASM sleep/cancel/refresh) is pending.
#[inline]
pub fn should_wake(name: &str) -> bool {
    name == "gpuid"
        && (is_input_ready()
            || crate::ui::input_queue::has_pending()
            || crate::ui::scene::wants_tick())
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

/// Drain input in process context, then re-enable level-triggered sources.
fn poll_and_rearm_input() {
    crate::cpu::display_proxy::touch_poll();
    crate::plic::unmask(0, crate::plic::VIRTIO_INPUT_IRQ);
    crate::plic::unmask(0, crate::plic::D1_TOUCH_IRQ);
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

fn is_main_screen_mode() -> bool {
    crate::ui::with_ui(|ui_mgr| ui_mgr.is_main_screen_mode()).unwrap_or(false)
}

/// Apply one input event to the scene. Owner only (caller must check).
/// Returns true if a main-screen button action was produced.
fn apply_input_event(event: crate::input::InputEvent, is_main_screen: bool) -> bool {
    use crate::input::{ABS_X, ABS_Y, EV_ABS};
    use crate::ui;

    if is_main_screen {
        if event.event_type == EV_ABS {
            match event.code {
                ABS_X => ui::set_cursor_pos(event.value, ui::get_cursor_pos().1),
                ABS_Y => ui::set_cursor_pos(ui::get_cursor_pos().0, event.value),
                _ => {}
            }
            false
        } else {
            ui::handle_main_screen_input(event).is_some()
        }
    } else {
        ui::with_ui(|ui_mgr| {
            ui_mgr.handle_input(event);
        });
        false
    }
}

/// Drain the WASM/syscall UI queue and apply it on hart 0.
///
/// Non-owners (including WASM on a secondary hart) must not call the
/// `static mut` scene mutators; this is the only apply path besides HID
/// that `gpuid` itself reads from the device.
fn drain_queued_ui() -> bool {
    use crate::ui;
    use crate::ui::input_queue;

    if !input_queue::is_owner() {
        return false;
    }

    if input_queue::take_cancel_apply() {
        ui::main_screen::request_cancel();
    }
    let is_main_screen = is_main_screen_mode();
    let mut had_button = false;
    while let Some(event) = input_queue::pop_input() {
        if apply_input_event(event, is_main_screen) {
            had_button = true;
        }
    }
    if input_queue::take_refresh() {
        ui::main_screen::refresh_terminal_output();
    }
    had_button
}

/// Copy MAIN_SCREEN cancel (Cancel button / ESC) into the guest flag.
/// Consumes the MAIN_SCREEN flag only while a WASM guest is the consumer,
/// so native `sys_should_cancel` still sees it for ELF commands.
fn mirror_guest_cancel() {
    use crate::ui;
    use crate::ui::input_queue;

    if input_queue::guest_ui_client() && ui::main_screen::should_cancel() {
        input_queue::publish_cancel();
    }
}

/// Apply queued UI work if this hart is the owner.
///
/// WASM host functions call this after enqueue so a hart-0 guest (which
/// occupies the owner hart and cannot schedule `gpuid`) still updates the
/// bitmap UI. Secondary harts return immediately.
pub fn apply_queued_if_owner() {
    if !crate::ui::input_queue::is_owner() {
        return;
    }
    if crate::ui::input_queue::has_pending() {
        let _ = drain_queued_ui();
        mirror_guest_cancel();
        publish_hdl_slice();
    }
}

/// Daemon service entry point for gpuid (GPU UI daemon)
/// Handles keyboard input and GPU display updates.
/// Runs at ~60 FPS when input is detected, otherwise polls less frequently.
///
/// Multi-hart safe: Uses display_proxy to delegate hardware access to Hart 0.
pub fn gpuid_service() {
    use crate::cpu::display_proxy;
    use crate::services::klogd::klog_info;
    use crate::ui;

    crate::perfstat::inc(crate::perfstat::id::GPUID_TICKS);

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
        publish_hdl_slice();

        // Initial render (flush will happen at end of function)
        ui::with_ui(|ui_mgr| {
            ui_mgr.mark_dirty();
            ui_mgr.render();
        });

        // Deferred flush at end of frame
        if crate::platform::d1_display::is_frame_dirty() {
            display_proxy::flush();
        }

        // Unmask HID so the first click after the splash is not lost.
        poll_and_rearm_input();

        klog_info("gpuid", "GUI transition complete");
        return;
    }

    // Always drain HID in process context and unmask the level-triggered
    // sources. Sleeping first when INPUT_READY was false left the IRQ masked
    // and the virtqueue undrained, so the GUI looked frozen after the first frame.
    clear_input_ready();
    poll_and_rearm_input();

    let mut had_button_action = drain_queued_ui();
    mirror_guest_cancel();

    let due_for_stats = stats_refresh_due();
    if !display_proxy::touch_has_events()
        && !crate::platform::d1_display::is_frame_dirty()
        && !due_for_stats
        && !ui::input_queue::has_pending()
        && !ui::scene::wants_tick()
    {
        crate::cpu::sched::sleep_current_ms(8);
        return;
    }

    // Check main screen mode once before processing events (optimization)
    let is_main_screen = is_main_screen_mode();

    // COALESCED event processing - drain all events, update state atomically
    // This prevents lag from intermediate mouse positions
    let mut had_input = false;

    while let Some(event) = display_proxy::touch_next_event() {
        had_input = true;
        if apply_input_event(event, is_main_screen) {
            had_button_action = true;
        }
    }
    mirror_guest_cancel();

        // Render cursor at FINAL position (after all events processed)
        if is_main_screen {
            // Browser HDL: CSS cursor. D1 / unpublished: last OpImage in the scene.

        // Check for GUI command completion (async polling)
        ui::main_screen::check_gui_command_completion();

        // Periodically update hardware stats (no input needed)
        if !had_input {
            ui::update_main_screen_hardware_stats();
        }
        publish_hdl_slice();
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
    use crate::cpu::display_proxy;
    use crate::ui;

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
        publish_hdl_slice();

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
    poll_and_rearm_input();
    let _ = drain_queued_ui();

    // Check main screen mode once before processing events
    let is_main_screen = is_main_screen_mode();

    // COALESCED event processing (same as gpuid_service)
    let mut had_input = false;
    while let Some(event) = display_proxy::touch_next_event() {
        had_input = true;
        let _ = apply_input_event(event, is_main_screen);
    }
    mirror_guest_cancel();

    // Render (no cursor - using browser's native cursor)
    if is_main_screen {
        // Check for GUI command completion (async polling)
        ui::main_screen::check_gui_command_completion();

        // Periodically update hardware stats (no input needed)
        if !had_input {
            ui::update_main_screen_hardware_stats();
        }
        publish_hdl_slice();
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

/// Encode + mailbox-publish (or raster_soft fallback) the retained desktop.
fn publish_hdl_slice() {
    if !is_main_screen_mode() {
        return;
    }
    crate::ui::scene::sync_from_owner();
    crate::ui::scene::publish_if_dirty();
}
