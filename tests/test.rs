#![no_std]
#![no_main]
#![feature(used_with_arg)]

extern crate alloc;

use alloc::{vec, vec::Vec};
use bare_test::{
    GetIrqConfig,
    async_std::time,
    fdt_parser::PciSpace,
    globals::global_val,
    irq::{IrqHandleResult, IrqInfo, IrqParam},
    mem::mmu::iomap,
    platform::fdt::GetPciIrqConfig,
    println,
};
use core::time::Duration;
use futures::FutureExt;
use log::{debug, info};
use pcie::*;
use usb_host::*;

#[bare_test::tests]
mod tests {
    use bare_test::irq::{IrqHandleResult, IrqParam};

    use super::*;

    #[test]
    fn test_cmd() {
        spin_on::spin_on(async {
            let info = get_usb_host();

            if let Some(irq) = &info.irq {
                for one in &irq.cfgs {
                    IrqParam {
                        irq_chip: irq.irq_parent,
                        cfg: one.clone(),
                    }
                    .register_builder(|irq| {
                        debug!("USB {:?}", irq);
                        IrqHandleResult::Handled
                    });
                }
            }

            let mut host = info.usb;

            host.init().await.unwrap();

            host.test_cmd().await.unwrap();

            debug!("usb cmd ok");
        });
    }
}

struct KernelImpl;

impl Kernel for KernelImpl {
    fn sleep<'a>(duration: Duration) -> futures::future::LocalBoxFuture<'a, ()> {
        time::sleep(duration).boxed_local()
    }
}

set_impl!(KernelImpl);

struct XhciInfo {
    usb: USBHost<Xhci>,
    irq: Option<IrqInfo>,
}

fn get_usb_host() -> XhciInfo {
    let fdt = match &global_val().platform_info {
        bare_test::globals::PlatformInfoKind::DeviceTree(fdt) => fdt,

        _ => panic!("unsupported platform"),
    };

    let fdt = fdt.get();
    let pcie = fdt
        .find_compatible(&["pci-host-ecam-generic"])
        .next()
        .unwrap()
        .into_pci()
        .unwrap();

    let mut pcie_regs = alloc::vec![];

    println!("pcie: {}", pcie.node.name);

    for reg in pcie.node.reg().unwrap() {
        println!("pcie reg: {:#x}", reg.address);
        pcie_regs.push(iomap((reg.address as usize).into(), reg.size.unwrap()));
    }

    let mut bar_alloc = SimpleBarAllocator::default();

    for range in pcie.ranges().unwrap() {
        info!("pcie range: {:?}", range);

        match range.space {
            PciSpace::Memory32 => bar_alloc.set_mem32(range.cpu_address as _, range.size as _),
            PciSpace::Memory64 => bar_alloc.set_mem64(range.cpu_address, range.size),
            _ => {}
        }
    }

    let base_vaddr = pcie_regs[0];

    info!("Init PCIE @{:?}", base_vaddr);

    let mut root = RootComplexGeneric::new(base_vaddr);

    for elem in root.enumerate(None, Some(bar_alloc)) {
        debug!("PCI {}", elem);

        if let Header::Endpoint(ep) = elem.header {
            ep.update_command(elem.root, |mut cmd| {
                cmd.remove(CommandRegister::INTERRUPT_DISABLE);
                cmd | CommandRegister::IO_ENABLE
                    | CommandRegister::MEMORY_ENABLE
                    | CommandRegister::BUS_MASTER_ENABLE
            });

            println!("irq_pin {:?}, {:?}", ep.interrupt_pin, ep.interrupt_line);

            if matches!(ep.device_type(), DeviceType::UsbController) {
                let bar_addr;
                let bar_size;
                match ep.bar {
                    pcie::BarVec::Memory32(bar_vec_t) => {
                        let bar0 = bar_vec_t[0].as_ref().unwrap();
                        bar_addr = bar0.address as usize;
                        bar_size = bar0.size as usize;
                    }
                    pcie::BarVec::Memory64(bar_vec_t) => {
                        let bar0 = bar_vec_t[0].as_ref().unwrap();
                        bar_addr = bar0.address as usize;
                        bar_size = bar0.size as usize;
                    }
                    pcie::BarVec::Io(_bar_vec_t) => todo!(),
                };

                println!("bar0: {:#x}", bar_addr);

                let addr = iomap(bar_addr.into(), bar_size);

                let irq = pcie.child_irq_info(
                    ep.address.bus(),
                    ep.address.device(),
                    ep.address.function(),
                    ep.interrupt_pin,
                );

                println!("irq: {irq:?}");

                return XhciInfo {
                    usb: USBHost::new(addr),
                    irq,
                };
            }
        }
    }

    for node in fdt.all_nodes() {
        if node.compatibles().any(|c| c.contains("xhci")) {
            println!("usb node: {}", node.name);
            let regs = node.reg().unwrap().collect::<Vec<_>>();
            println!("usb regs: {:?}", regs);

            let addr = iomap(
                (regs[0].address as usize).into(),
                regs[0].size.unwrap_or(0x1000),
            );

            let irq = node.irq_info();

            return XhciInfo {
                usb: USBHost::new(addr),
                irq,
            };
        }
    }

    panic!("no xhci found");
}
