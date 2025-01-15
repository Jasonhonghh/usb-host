use dma_api::DVec;
pub use dma_api::Direction;

use crate::err::*;

const TRB_LEN: usize = 4;

#[derive(Clone)]
#[repr(transparent)]
pub struct TrbData([u32; TRB_LEN]);

pub struct Ring {
    link: bool,
    pub trbs: DVec<TrbData>,
    pub i: usize,
    pub cycle: bool,
}

impl Ring {
    pub fn new(len: usize, link: bool, direction: Direction) -> Result<Self> {
        let trbs = DVec::zeros(len, 64, direction).ok_or(USBError::NoMemory)?;

        Ok(Self {
            link,
            trbs,
            i: 0,
            cycle: link,
        })
    }

    pub fn len(&self) -> usize {
        self.trbs.len()
    }

    fn get_trb(&self) -> Option<TrbData> {
        self.trbs.get(self.i)
    }

    pub fn bus_addr(&self) -> u64 {
        self.trbs.bus_addr()
    }
}
