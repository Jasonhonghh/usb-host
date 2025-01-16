use core::{
    future::Future,
    sync::atomic::{fence, Ordering},
    task::Poll,
};

use alloc::{collections::btree_map::BTreeMap, sync::Arc};
use dma_api::DVec;
use futures::{future::LocalBoxFuture, FutureExt};
use log::debug;
use spin::{mutex::Mutex, rwlock::RwLock};
use xhci::ring::trb::event::{Allowed, CompletionCode};

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
    results: RwLock<BTreeMap<u64, Arc<Mutex<Option<Allowed>>>>>,
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

        Ok(Self {
            ring,
            ste,
            results: RwLock::new(Default::default()),
        })
    }

    pub fn wait_result(&mut self, trb_addr: u64) -> LocalBoxFuture<'_, Allowed> {
        {
            let mut guard = self.results.write();
            guard.insert(trb_addr, Arc::new(Mutex::new(None)));
        }

        EventWaiter {
            trb_addr,
            ring: self,
        }
        .boxed_local()
    }

    pub fn clean_events(&mut self) {
        while let Some((allowed, cycle)) = self.next() {
            match allowed {
                Allowed::CommandCompletion(c) =>{
                    let addr = c.command_trb_pointer();

                    
                },
                _ => {
                    debug!("unhandled event {:?}", allowed);
                }
            }
        }
    }

    /// 完成一次循环返回 true
    pub fn next(&mut self) -> Option<(Allowed, bool)> {
        let (data, flag) = self.ring.current_data();

        let allowed = Allowed::try_from(data.to_raw()).ok()?;

        if flag != allowed.cycle_bit() {
            return None;
        }

        fence(Ordering::SeqCst);

        let cycle = self.ring.inc_deque();
        Some((allowed, cycle))
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

struct EventWaiter<'a> {
    trb_addr: u64,
    ring: &'a EventRing,
}

impl Future for EventWaiter<'_> {
    type Output = Allowed;

    fn poll(
        self: core::pin::Pin<&mut Self>,
        _cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        let addr = self.trb_addr;
        let entry = {
            unsafe { &mut *self.ring.results.as_mut_ptr() }
                .get(&addr)
                .unwrap()
                .clone()
        };

        let mut g = entry.lock();
        match g.take() {
            Some(v) => Poll::Ready(v),
            None => Poll::Pending,
        }
    }
}
