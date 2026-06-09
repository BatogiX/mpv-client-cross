use mpv_client_sys::{mpv_error, mpv_error_MPV_ERROR_GENERIC};
use std::{ffi::NulError, fmt, str::Utf8Error};

use crate::Handle;

#[derive(Debug)]
pub struct Error(mpv_error);
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    #[must_use]
    pub const fn new(error: mpv_error) -> Self {
        Self(error)
    }
}

impl From<NulError> for Error {
    fn from(_: NulError) -> Self {
        Self::new(mpv_error_MPV_ERROR_GENERIC)
    }
}

impl From<Utf8Error> for Error {
    fn from(_: Utf8Error) -> Self {
        Self::new(mpv_error_MPV_ERROR_GENERIC)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let err = Handle::error_string(self.0);
        write!(f, "[{}] {}", self.0, err)
    }
}
