use thiserror::Error;

#[derive(Error, Debug)]
pub enum USBError {
    #[error("unknown usb error")]
    Unknown,
    #[error("not initialized")]
    NotInitialized,
    #[error("no memory")]
    NoMemory,
}

pub type Result<T = ()> = core::result::Result<T, USBError>;
