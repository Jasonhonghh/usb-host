#![no_std]

extern crate alloc;

use core::time::Duration;

mod host;
pub use host::*;

pub trait Kernel {
    fn sleep(duration: Duration);
}

pub(crate) fn sleep(duration: Duration) {
    extern "Rust" {
        fn _usb_host_sleep(duration: Duration);
    }

    unsafe {
        _usb_host_sleep(duration);
    }
}

#[macro_export]
macro_rules! set_impl {
    ($t: ty) => {
        #[no_mangle]
        unsafe fn _usb_host_sleep(duration: core::time::Duration) {
            <$t as $crate::Kernel>::sleep(duration)
        }
    };
}
