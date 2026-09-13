//! Block device used by SFS. Virt: virtio-blk. D1: SMHC/MMC.

pub trait SectorIo {
    fn read_sector(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), &'static str>;
    fn write_sector(&mut self, sector: u64, buf: &[u8]) -> Result<(), &'static str>;
    fn capacity(&self) -> u64;
}

pub enum BlockDeviceState {
    #[cfg(feature = "d1")]
    Mmc(crate::platform::d1_mmc::D1Mmc),
    #[cfg(not(feature = "d1"))]
    Virtio(crate::device::virtio_blk::VirtioBlk),
}

impl BlockDeviceState {
    pub fn read_sector(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), &'static str> {
        match self {
            #[cfg(feature = "d1")]
            Self::Mmc(d) => d.read_sector(sector, buf),
            #[cfg(not(feature = "d1"))]
            Self::Virtio(d) => d.read_sector(sector, buf),
        }
    }

    pub fn write_sector(&mut self, sector: u64, buf: &[u8]) -> Result<(), &'static str> {
        match self {
            #[cfg(feature = "d1")]
            Self::Mmc(d) => d.write_sector(sector, buf),
            #[cfg(not(feature = "d1"))]
            Self::Virtio(d) => d.write_sector(sector, buf),
        }
    }

    pub fn capacity(&self) -> u64 {
        match self {
            #[cfg(feature = "d1")]
            Self::Mmc(d) => d.capacity(),
            #[cfg(not(feature = "d1"))]
            Self::Virtio(d) => d.capacity(),
        }
    }
}

impl SectorIo for BlockDeviceState {
    fn read_sector(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), &'static str> {
        BlockDeviceState::read_sector(self, sector, buf)
    }
    fn write_sector(&mut self, sector: u64, buf: &[u8]) -> Result<(), &'static str> {
        BlockDeviceState::write_sector(self, sector, buf)
    }
    fn capacity(&self) -> u64 {
        BlockDeviceState::capacity(self)
    }
}

#[cfg(feature = "d1")]
pub type D1Mmc = crate::platform::d1_mmc::D1Mmc;
