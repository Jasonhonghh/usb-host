use alloc::vec::Vec;
use dma_api::{DBox, DVec};
use xhci::context::{Device, Device64Byte, Input64Byte};

use super::ring::Ring;
use crate::err::*;

pub struct DeviceContextList {
    pub dcbaa: DVec<u64>,
    pub device_context_list: Vec<DeviceContext>,
    max_slots: usize,
}

pub struct DeviceContext {
    pub out: DBox<Device64Byte>,
    pub input: DBox<Input64Byte>,
    pub transfer_rings: Vec<Ring>,
}

impl DeviceContext {
    fn new() -> Result<Self> {
        let out = DBox::zero(dma_api::Direction::ToDevice).ok_or(USBError::NoMemory)?;
        let input = DBox::zero(dma_api::Direction::FromDevice).ok_or(USBError::NoMemory)?;
        Ok(Self {
            out,
            input,
            transfer_rings: Vec::new(),
        })
    }
}

impl DeviceContextList {
    pub fn new(max_slots: usize) -> Result<Self> {
        let dcbaa = DVec::zeros(256, 0x1000, dma_api::Direction::Bidirectional)
            .ok_or(USBError::NoMemory)?;

        Ok(Self {
            dcbaa,
            device_context_list: Vec::new(),
            max_slots,
        })
    }

    pub fn new_slot(
        &mut self,
        slot: usize,
        hub: usize,
        port: usize,
        num_ep: usize, // cannot lesser than 0, and consider about alignment, use usize
    ) -> Result {
        if slot > self.max_slots {
            Err(USBError::SlotLimitReached)?;
        }

        Ok(())
    }
}
