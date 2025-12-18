use core::sync::atomic::Ordering;
use crate::dtb::{self, DTB_ADDR};

pub fn init_dtb() {
   let dtb_addr = DTB_ADDR.load(Ordering::Acquire);
    if dtb_addr != 0 {
        dtb::init(dtb_addr);
    }
}