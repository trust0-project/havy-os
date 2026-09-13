pub fn init_touch() {
    #[cfg(feature = "d1")]
    {
        let _ = crate::platform::d1_touch::init();
    }
    #[cfg(not(feature = "d1"))]
    {
        let _ = crate::virtio_input::init();
    }
}
