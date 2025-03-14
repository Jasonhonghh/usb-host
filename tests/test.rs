#![no_std]
#![no_main]
#![feature(used_with_arg)]

extern crate alloc;

use alloc::vec::Vec;
use bare_test::{
    GetIrqConfig,
    async_std::time,
    fdt_parser::PciSpace,
    globals::global_val,
    irq::{IrqHandleResult, IrqInfo, IrqParam},
    mem::{
        Align,
        mmu::{iomap, page_size},
    },
    platform::fdt::GetPciIrqConfig,
    println,
};
use core::{cell::UnsafeCell, time::Duration};
use futures::FutureExt;
use log::*;
use pcie::*;
use usb_host::*;

struct Host(UnsafeCell<USBHost<Xhci>>);
unsafe impl Send for Host {}
unsafe impl Sync for Host {}

#[bare_test::tests]
mod tests {
    use core::hint::spin_loop;

    use alloc::sync::Arc;
    use bare_test::{
        irq::{IrqHandleResult, IrqParam},
        platform::cpu_id,
        task::TaskConfig,
        time::sleep,
    };
    use log::warn;

    use super::*;

    #[test]
    fn test_cmd() {
        let info = get_usb_host();
        let host = info.usb;

        let host = Arc::new(Host(UnsafeCell::new(host)));
        bare_test::time::after(Duration::from_secs(5), {
            let host = host.clone();
            move || {
                debug!("test timer");

                unsafe {
                    (&mut *host.0.get()).handle_irq();
                }
            }
        });

        if let Some(irq) = &info.irq {
            for one in &irq.cfgs {
                IrqParam {
                    intc: irq.irq_parent,
                    cfg: one.clone(),
                }
                .register_builder({
                    let host = host.clone();
                    move |irq| {
                        unsafe {
                            (&mut *host.0.get()).handle_irq();
                        }
                        IrqHandleResult::Handled
                    }
                })
                .register();
            }
        }

        spin_on::spin_on(async move {
            let host = unsafe { &mut *host.0.get() };

            host.init().await.unwrap();

            debug!("usb cmd test");

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

    fn page_size() -> usize {
        page_size()
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
        .find_compatible(&["pci-host-ecam-generic", "brcm,bcm2711-pcie"])
        .next()
        .unwrap()
        .into_pci()
        .unwrap();

    let mut pcie_regs = alloc::vec![];

    println!("pcie: {}", pcie.node.name);

    for reg in pcie.node.reg().unwrap() {
        println!(
            "pcie reg: {:#x}, bus: {:#x}",
            reg.address, reg.child_bus_address
        );
        let size = reg.size.unwrap_or_default().align_up(0x1000);

        pcie_regs.push(iomap((reg.address as usize).into(), size));
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

    // for elem in root.enumerate_keep_bar(None) {
    for elem in root.enumerate(None, Some(bar_alloc)) {
        debug!("PCI {}", elem);

        if let Header::Endpoint(mut ep) = elem.header {
            ep.update_command(elem.root, |mut cmd| {
                cmd.remove(CommandRegister::INTERRUPT_DISABLE);
                cmd | CommandRegister::IO_ENABLE
                    | CommandRegister::MEMORY_ENABLE
                    | CommandRegister::BUS_MASTER_ENABLE
            });

            for cap in &mut ep.capabilities {
                match cap {
                    PciCapability::Msi(msi_capability) => {
                        msi_capability.set_enabled(false, &mut *elem.root);
                    }
                    PciCapability::MsiX(msix_capability) => {
                        msix_capability.set_enabled(false, &mut *elem.root);
                    }
                    _ => {}
                }
            }

            println!("irq_pin {:?}, {:?}", ep.interrupt_pin, ep.interrupt_line);

            if matches!(ep.device_type(), DeviceType::UsbController) {
                let bar_addr;
                let mut bar_size;
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
                println!("bar0 size: {:#x}", bar_size);
                bar_size = bar_size.align_up(0x1000);
                println!("bar0 size algin: {:#x}", bar_size);

                let addr = iomap(bar_addr.into(), bar_size);
                trace!("pin {:?}", ep.interrupt_pin);

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
