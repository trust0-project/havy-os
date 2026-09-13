use crate::{boot::console::GpuConsole, platform};

pub fn init_gpu() {
    crate::ui::hdl_mailbox::init();
    if let Ok(()) = platform::d1_display::init() {
        GpuConsole::set_available(true);
        crate::ui::boot::init();
        unsafe {
            crate::ui::UI_MANAGER = Some(crate::ui::UiManager::new());
        }
    }
}
