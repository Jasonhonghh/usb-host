use core::{num::NonZeroUsize, ptr::NonNull, time::Duration};

use context::ScratchpadBufferArray;
use future::LocalBoxFuture;
use futures::prelude::*;
use log::{debug, info};
use ring::{Ring, TrbData};
use xhci::{
    accessor::Mapper,
    ring::trb::{self, command, event::CommandCompletion},
};

mod context;
mod event;
mod ring;

use super::Controller;
use crate::{err::*, sleep};

type Registers = xhci::Registers<MemMapper>;
type RegistersExtList = xhci::extended_capabilities::List<MemMapper>;
type SupportedProtocol = xhci::extended_capabilities::XhciSupportedProtocol<MemMapper>;

pub struct Xhci {
    mmio_base: NonNull<u8>,
    data: Option<Data>,
}

impl Xhci {
    pub fn new(mmio_base: NonNull<u8>) -> Self {
        Self {
            mmio_base,
            data: None,
        }
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

    fn setup_max_device_slots(&mut self) -> u8 {
        let mut regs = self.regs();
        let max_slots = regs
            .capability
            .hcsparams1
            .read_volatile()
            .number_of_device_slots();

        regs.operational.config.update_volatile(|r| {
            r.set_max_device_slots_enabled(max_slots);
        });

        debug!("Max device slots: {}", max_slots);

        max_slots
    }

    fn setup_dcbaap(&mut self) -> Result {
        let dcbaa_addr = self.data()?.dev_list.dcbaa.bus_addr();
        debug!("DCBAAP: {:X}", dcbaa_addr);
        self.regs().operational.dcbaap.update_volatile(|r| {
            r.set(dcbaa_addr);
        });

        Ok(())
    }

    fn set_cmd_ring(&mut self) -> Result {
        let crcr = self.data()?.cmd.trbs.bus_addr();
        let cycle = self.data()?.cmd.cycle;

        debug!("CRCR: {:X}", crcr);
        self.regs().operational.crcr.update_volatile(|r| {
            r.set_command_ring_pointer(crcr);
            if cycle {
                r.set_ring_cycle_state();
            } else {
                r.clear_ring_cycle_state();
            }
        });

        Ok(())
    }

    fn init_irq(&mut self) -> Result {
        debug!("Disable interrupts");
        let mut regs = self.regs();

        regs.operational.usbcmd.update_volatile(|r| {
            r.clear_interrupter_enable();
        });

        let erstz = self.data()?.event.len();
        let erdp = self.data()?.event.erdp();
        let erstba = self.data()?.event.erstba();

        let mut ir0 = regs.interrupter_register_set.interrupter_mut(0);
        {
            debug!("ERSTZ: {:x}", erstz);
            ir0.erstsz.update_volatile(|r| r.set(erstz as _));

            debug!("ERDP: {:x}", erdp);

            ir0.erdp.update_volatile(|r| {
                r.set_event_ring_dequeue_pointer(erdp);
            });

            debug!("ERSTBA: {:X}", erstba);

            ir0.erstba.update_volatile(|r| {
                r.set(erstba);
            });
            ir0.imod.update_volatile(|im| {
                im.set_interrupt_moderation_interval(0);
                im.set_interrupt_moderation_counter(0);
            });

            debug!("Enabling primary interrupter.");
            ir0.iman.update_volatile(|im| {
                im.set_interrupt_enable();
            });
        }

        // self.setup_scratchpads(buf_count);

        Ok(())
    }

    fn setup_scratchpads(&mut self) -> Result {
        let scratchpad_buf_arr = {
            let buf_count = {
                let count = self
                    .regs()
                    .capability
                    .hcsparams2
                    .read_volatile()
                    .max_scratchpad_buffers();
                debug!("Scratch buf count: {}", count);
                count
            };
            if buf_count == 0 {
                return Ok(());
            }
            let scratchpad_buf_arr = ScratchpadBufferArray::new(buf_count as _)?;

            let bus_addr = scratchpad_buf_arr.bus_addr();

            self.data()?.dev_list.dcbaa.set(0, bus_addr);

            debug!("Setting up {} scratchpads, at {:#0x}", buf_count, bus_addr);
            scratchpad_buf_arr
        };

        self.data()?.scratchpad_buf_arr = Some(scratchpad_buf_arr);

        Ok(())
    }

    async fn start(&mut self) -> Result {
        let mut regs = self.regs();
        debug!("Start run");

        regs.operational.usbcmd.update_volatile(|r| {
            r.set_run_stop();
        });

        while regs.operational.usbsts.read_volatile().hc_halted() {
            sleep(Duration::from_millis(10)).await;
        }

        info!("Running");

        regs.doorbell.update_volatile_at(0, |r| {
            r.set_doorbell_stream_id(0);
            r.set_doorbell_target(0);
        });

        Ok(())
    }

    async fn post_cmd(&mut self, trb: command::Allowed) -> Result {
        let trb_addr = self.data()?.cmd.enque_command(trb);

        self.regs().doorbell.update_volatile_at(0, |r| {
            r.set_doorbell_stream_id(0);
            r.set_doorbell_target(0);
        });

        let res = self.data()?.event.wait_result(trb_addr).await?;

        if let trb::event::Allowed::CommandCompletion(c) = res {
        } else {
            panic!("Invalid event type")
        }

        Ok(())
    }

    fn handle_event(&mut self) {
        while let Some((allowed, cycle)) = self.data().as_mut().unwrap().event.next() {
            match allowed {
                trb::event::Allowed::TransferEvent(transfer_event) => todo!(),
                trb::event::Allowed::CommandCompletion(command_completion) => todo!(),
                trb::event::Allowed::PortStatusChange(port_status_change) => todo!(),
                trb::event::Allowed::BandwidthRequest(bandwidth_request) => todo!(),
                trb::event::Allowed::Doorbell(doorbell) => todo!(),
                trb::event::Allowed::HostController(host_controller) => todo!(),
                trb::event::Allowed::DeviceNotification(device_notification) => todo!(),
                trb::event::Allowed::MfindexWrap(mfindex_wrap) => todo!(),
            }
        }
    }

    fn data(&mut self) -> Result<&mut Data> {
        self.data.as_mut().ok_or(USBError::NotInitialized)
    }
}

struct Data {
    dev_list: context::DeviceContextList,
    cmd: Ring,
    event: event::EventRing,
    scratchpad_buf_arr: Option<ScratchpadBufferArray>,
}

impl Data {
    fn new(max_slots: usize) -> Result<Self> {
        Ok(Self {
            dev_list: context::DeviceContextList::new(max_slots)?,
            cmd: Ring::new(
                0x1000 / size_of::<TrbData>(),
                true,
                dma_api::Direction::Bidirectional,
            )?,
            event: event::EventRing::new()?,
            scratchpad_buf_arr: None,
        })
    }
}

impl Controller for Xhci {
    fn init(&mut self) -> LocalBoxFuture<'_, Result> {
        async {
            self.chip_hardware_reset().await?;
            let max_slots = self.setup_max_device_slots();
            self.data = Some(Data::new(max_slots as _)?);
            self.setup_dcbaap()?;
            self.set_cmd_ring()?;
            self.init_irq()?;
            self.setup_scratchpads()?;
            self.start().await?;

            Ok(())
        }
        .boxed_local()
    }

    fn test_cmd(&mut self) -> LocalBoxFuture<'_, Result> {
        async { Ok(()) }.boxed_local()
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
