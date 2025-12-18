use crate::{RwLock, Spinlock, lock::state::{
    blk::BlockDeviceState,
    cwd::CwdState,
    fs::FileSystemState,
    log::LogBufferState,
    net::NetState,
    output::OutputCaptureState,
    ping::PingState,
    shell::ShellCmdState,
    tail::TailFollowState,
    waitq::WaitQueueState,
}};

// Re-export constants from state submodules
pub(crate) use crate::lock::state::cwd::CWD_MAX_LEN;
pub(crate) use crate::lock::state::output::OUTPUT_BUFFER_SIZE;

pub(crate) static CWD_STATE: Spinlock<CwdState> = Spinlock::new(CwdState::new());
pub(crate) static NET_STATE: Spinlock<Option<NetState>> = Spinlock::new(None);
pub(crate) static FS_STATE: RwLock<Option<FileSystemState>> = RwLock::new(None);
pub(crate) static TIMER_WAITQ: Spinlock<Option<WaitQueueState>> = Spinlock::new(None);
pub(crate) static IO_WAITQ: Spinlock<Option<WaitQueueState>> = Spinlock::new(None);
pub(crate) static IPC_WAITQ: Spinlock<Option<WaitQueueState>> = Spinlock::new(None);
pub(crate) static LOG_BUFFER: Spinlock<LogBufferState> = Spinlock::new(LogBufferState::new());
pub(crate) static PING_STATE: Spinlock<Option<PingState>> = Spinlock::new(None);
pub(crate) static COMMAND_RUNNING: Spinlock<bool> = Spinlock::new(false);
pub(crate) static TAIL_FOLLOW_STATE: Spinlock<TailFollowState> = Spinlock::new(TailFollowState::new());
pub(crate) static BLK_DEV: RwLock<Option<BlockDeviceState>> = RwLock::new(None);
pub(crate) static OUTPUT_CAPTURE: Spinlock<OutputCaptureState> = Spinlock::new(OutputCaptureState::new());
pub(crate) static SHELL_CMD_STATE: Spinlock<ShellCmdState> = Spinlock::new(ShellCmdState::new());

