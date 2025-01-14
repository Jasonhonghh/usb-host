#![no_std]

extern crate alloc;

use core::time::Duration;

pub mod err;
mod host;

pub use futures::future::LocalBoxFuture;
pub use host::*;

pub trait Kernel {
    fn sleep<'a>(duration: Duration) -> LocalBoxFuture<'a, ()>;
}

pub(crate) async fn sleep(duration: Duration) {
    extern "Rust" {
        fn _usb_host_sleep<'a>(duration: Duration) -> LocalBoxFuture<'a, ()>;
    }

    unsafe {
        _usb_host_sleep(duration).await;
    }
}

#[macro_export]
macro_rules! set_impl {
    ($t: ty) => {
        #[no_mangle]
        unsafe fn _usb_host_sleep<'a>(
            duration: core::time::Duration,
        ) -> $crate::LocalBoxFuture<'a, ()> {
            <$t as $crate::Kernel>::sleep(duration)
        }
    };
}
