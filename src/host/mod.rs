use core::ptr::NonNull;

use futures::future::LocalBoxFuture;

pub mod xhci;

use crate::err::*;
pub use xhci::Xhci;

pub struct USBHost<C>
where
    C: Controller,
{
    ctrl: C,
}

impl<C> From<C> for USBHost<C>
where
    C: Controller,
{
    fn from(value: C) -> Self {
        Self { ctrl: value }
    }
}

impl USBHost<Xhci> {
    pub fn new(reg_base: NonNull<u8>) -> Self {
        Self::from(Xhci::new(reg_base))
    }

    pub async fn open(&mut self) -> Result {
        self.ctrl.open().await
    }
}

pub trait Controller {
    fn open(&mut self) -> LocalBoxFuture<'_, Result>;
}
