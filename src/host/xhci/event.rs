use dma_api::DVec;

use super::ring::Ring;
use crate::err::*;

#[repr(C)]
pub struct EventRingSte {
    pub addr: u64,
    pub size: u16,
    _reserved: [u8; 6],
}

pub struct EventRing {
    pub ring: Ring,
    pub ste: DVec<EventRingSte>,
}

impl EventRing {
    pub fn new() -> Result<Self> {
        let ring = Ring::new(256, true, dma_api::Direction::Bidirectional)?;

        let mut ste =
            DVec::zeros(1, 64, dma_api::Direction::Bidirectional).ok_or(USBError::NoMemory)?;

        let ste0 = EventRingSte {
            addr: ring.trbs.bus_addr(),
            size: ring.len() as _,
            _reserved: [0; 6],
        };

        ste.set(0, ste0);

        Ok(Self { ring, ste })
    }

    pub fn erdp(&self) -> u64 {
        self.ring.bus_addr() & 0xFFFF_FFFF_FFFF_FFF0
    }
    pub fn erstba(&self) -> u64 {
        self.ste.bus_addr()
    }

    pub fn len(&self) -> usize {
        self.ste.len()
    }
}
