use core::sync::atomic::{AtomicBool, Ordering};

use crate::boot::{
    cpu::init_cpu, 
    dtb::init_dtb, 
    gpu::init_gpu, 
    logger::init_logger, 
    memory::init_memory, 
    network::init_network, 
    storage::init_storage, 
    touch::init_touch, 
    services::init_services
};


pub mod console;
pub mod storage;
pub mod network;
pub mod logger;
pub mod cpu;
pub mod memory;
pub mod gpu;
pub mod dtb;
pub mod touch;
pub mod services;

pub(crate) static BOOT_READY: AtomicBool = AtomicBool::new(false);

pub fn init_boot() {
    init_logger();
    init_dtb();
    init_gpu();
    init_cpu();
    init_memory();
    init_storage();
    init_network();
    init_touch();
    init_services();
    BOOT_READY.store(true, Ordering::Release);
}