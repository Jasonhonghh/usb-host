#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(bare_test::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use core::time::Duration;

use alloc::vec::Vec;
use bare_test::{
    driver::device_tree::get_device_tree, fdt::PciSpace, mem::mmu::iomap, println, time::delay,
};
use futures::FutureExt;
use log::{debug, info};
use pcie::*;
use usb_host::*;

bare_test::test_setup!();

#[test_case]
fn test_work() {
    spin_on::spin_on(async {
        let mut host = get_usb_host();

        host.init().await.unwrap();

        debug!("usb init ok");
    });
}

struct KernelImpl;

impl Kernel for KernelImpl {
    fn sleep<'a>(duration: Duration) -> futures::future::LocalBoxFuture<'a, ()> {
        delay(duration).boxed_local()
    }
}

set_impl!(KernelImpl);

fn get_usb_host() -> USBHost<Xhci> {
    let fdt = get_device_tree().unwrap();
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
            ep.update_command(elem.root, |cmd| {
                cmd | CommandRegister::IO_ENABLE
                    | CommandRegister::MEMORY_ENABLE
                    | CommandRegister::BUS_MASTER_ENABLE
            });

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

                return USBHost::new(addr);
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

            return USBHost::new(addr);
        }
    }

    panic!("no xhci found");
}
