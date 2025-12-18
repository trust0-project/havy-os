pub(crate) const OUTPUT_BUFFER_SIZE: usize = 4096;

/// Output capture state for redirection
pub(crate) struct OutputCaptureState {
    pub(crate) buffer: [u8; OUTPUT_BUFFER_SIZE],
    pub(crate) len: usize,
    pub(crate) capturing: bool,
}

impl OutputCaptureState {
    pub(crate) const fn new() -> Self {
        Self {
            buffer: [0u8; OUTPUT_BUFFER_SIZE],
            len: 0,
            capturing: false,
        }
    }
}

// Type alias for backwards compatibility
pub(crate) type OutputCapture = OutputCaptureState;
