use crate::{boot::console::GpuConsole, platform};

pub fn init_gpu() {
    if let Ok(()) = platform::d1_display::init() {
        GpuConsole::set_available(true);
        crate::ui::boot::init();
        unsafe {
            crate::ui::UI_MANAGER = Some(crate::ui::UiManager::new());
        }
        platform::d1_touch::init();
    }
}