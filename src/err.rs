use thiserror::Error;

#[derive(Error, Debug)]
pub enum USBError {
    #[error("unknown usb error")]
    Unknown,
}

pub type Result<T = ()> = core::result::Result<T, USBError>;
