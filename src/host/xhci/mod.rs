use core::{num::NonZeroUsize, ptr::NonNull, time::Duration};

use future::LocalBoxFuture;
use futures::prelude::*;
use log::debug;
use xhci::accessor::Mapper;

mod context;
mod ring;

use super::Controller;
use crate::{err::*, sleep};

type Registers = xhci::Registers<MemMapper>;
type RegistersExtList = xhci::extended_capabilities::List<MemMapper>;
type SupportedProtocol = xhci::extended_capabilities::XhciSupportedProtocol<MemMapper>;

pub struct Xhci {
    mmio_base: NonNull<u8>,
}

impl Xhci {
    pub fn new(mmio_base: NonNull<u8>) -> Self {
        Self { mmio_base }
    }

    fn regs(&self) -> Registers {
        let mapper = MemMapper {};
        unsafe { Registers::new(self.mmio_base.as_ptr() as usize, mapper) }
    }

    async fn chip_hardware_reset(&mut self) -> Result {
        debug!("Reset begin ...");
        let mut regs = self.regs();
        regs.operational.usbcmd.update_volatile(|c| {
            c.clear_run_stop();
        });

        while !regs.operational.usbsts.read_volatile().hc_halted() {
            sleep(Duration::from_millis(10)).await;
        }

        debug!("Halted");
        let o = &mut regs.operational;
        debug!("Wait for ready...");
        while o.usbsts.read_volatile().controller_not_ready() {
            sleep(Duration::from_millis(10)).await;
        }
        debug!("Ready");

        o.usbcmd.update_volatile(|f| {
            f.set_host_controller_reset();
        });

        debug!("Reset HC");
        while o.usbcmd.read_volatile().host_controller_reset()
            || o.usbsts.read_volatile().controller_not_ready()
        {
            sleep(Duration::from_millis(10)).await;
        }
        debug!("Reset finish");

        Ok(())
    }
}

impl Controller for Xhci {
    fn init(&mut self) -> LocalBoxFuture<'_, Result> {
        async {
            self.chip_hardware_reset().await?;

            Ok(())
        }
        .boxed_local()
    }
}

#[derive(Clone, Copy)]
pub struct MemMapper;
impl Mapper for MemMapper {
    unsafe fn map(&mut self, phys_start: usize, _bytes: usize) -> NonZeroUsize {
        NonZeroUsize::new_unchecked(phys_start)
    }
    fn unmap(&mut self, _virt_start: usize, _bytes: usize) {}
}
