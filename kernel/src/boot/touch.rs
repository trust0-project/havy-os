use crate::{ platform};

pub fn init_touch() {
    platform::d1_touch::init();
}