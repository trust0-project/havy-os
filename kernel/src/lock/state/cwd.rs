// --- CURRENT WORKING DIRECTORY ------------------------------------------------──
pub(crate) const CWD_MAX_LEN: usize = 128;

/// Current working directory state
pub(crate) struct CwdState {
    pub(crate) path: [u8; CWD_MAX_LEN],
    pub(crate) len: usize,
}

impl CwdState {
    pub(crate) const fn new() -> Self {
        let mut path = [0u8; CWD_MAX_LEN];
        path[0] = b'/';
        Self { path, len: 1 }
    }
}