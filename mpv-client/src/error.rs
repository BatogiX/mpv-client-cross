use crate::{Format, Handle};
use mpv_client_sys::{
    mpv_error, mpv_error_MPV_ERROR_AO_INIT_FAILED, mpv_error_MPV_ERROR_COMMAND, mpv_error_MPV_ERROR_EVENT_QUEUE_FULL,
    mpv_error_MPV_ERROR_GENERIC, mpv_error_MPV_ERROR_INVALID_PARAMETER, mpv_error_MPV_ERROR_LOADING_FAILED,
    mpv_error_MPV_ERROR_NOMEM, mpv_error_MPV_ERROR_NOT_IMPLEMENTED, mpv_error_MPV_ERROR_NOTHING_TO_PLAY,
    mpv_error_MPV_ERROR_OPTION_ERROR, mpv_error_MPV_ERROR_OPTION_FORMAT, mpv_error_MPV_ERROR_OPTION_NOT_FOUND,
    mpv_error_MPV_ERROR_PROPERTY_ERROR, mpv_error_MPV_ERROR_PROPERTY_FORMAT, mpv_error_MPV_ERROR_PROPERTY_NOT_FOUND,
    mpv_error_MPV_ERROR_PROPERTY_UNAVAILABLE, mpv_error_MPV_ERROR_SUCCESS, mpv_error_MPV_ERROR_UNINITIALIZED,
    mpv_error_MPV_ERROR_UNKNOWN_FORMAT, mpv_error_MPV_ERROR_UNSUPPORTED, mpv_error_MPV_ERROR_VO_INIT_FAILED,
};
use std::{ffi::NulError, result, str::Utf8Error};
use thiserror::Error as ThisError;

pub type Result<T> = result::Result<T, Error>;

#[derive(Debug, ThisError)]
pub enum Error {
    #[error("{} ({})", Handle::error_string(*.0 as i32), *.0 as i32, )]
    MpvKnown(MpvError),

    #[error("{} ({})", Handle::error_string(*.0), .0,)]
    MpvUnknown(i32),

    #[error("{0}")]
    Nul(#[from] NulError),

    #[error("{0}")]
    Utf8(#[from] Utf8Error),

    #[error("format mismatch ({0})")]
    FormatMismatch(Format),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum MpvError {
    Success = mpv_error_MPV_ERROR_SUCCESS,
    EventQueueFull = mpv_error_MPV_ERROR_EVENT_QUEUE_FULL,
    Nomem = mpv_error_MPV_ERROR_NOMEM,
    Uninitialized = mpv_error_MPV_ERROR_UNINITIALIZED,
    InvalidParameter = mpv_error_MPV_ERROR_INVALID_PARAMETER,
    OptionNotFound = mpv_error_MPV_ERROR_OPTION_NOT_FOUND,
    OptionFormat = mpv_error_MPV_ERROR_OPTION_FORMAT,
    OptionError = mpv_error_MPV_ERROR_OPTION_ERROR,
    PropertyNotFound = mpv_error_MPV_ERROR_PROPERTY_NOT_FOUND,
    PropertyFormat = mpv_error_MPV_ERROR_PROPERTY_FORMAT,
    PropertyUnavailable = mpv_error_MPV_ERROR_PROPERTY_UNAVAILABLE,
    PropertyError = mpv_error_MPV_ERROR_PROPERTY_ERROR,
    Command = mpv_error_MPV_ERROR_COMMAND,
    LoadingFailed = mpv_error_MPV_ERROR_LOADING_FAILED,
    AoInitFailed = mpv_error_MPV_ERROR_AO_INIT_FAILED,
    VoInitFailed = mpv_error_MPV_ERROR_VO_INIT_FAILED,
    NothingToPlay = mpv_error_MPV_ERROR_NOTHING_TO_PLAY,
    UnknownFormat = mpv_error_MPV_ERROR_UNKNOWN_FORMAT,
    Unsupported = mpv_error_MPV_ERROR_UNSUPPORTED,
    NotImplemented = mpv_error_MPV_ERROR_NOT_IMPLEMENTED,
    Generic = mpv_error_MPV_ERROR_GENERIC,
}

impl From<mpv_error> for Error {
    fn from(code: mpv_error) -> Self {
        match code {
            mpv_error_MPV_ERROR_SUCCESS => Self::MpvKnown(MpvError::Success),
            mpv_error_MPV_ERROR_NOMEM => Self::MpvKnown(MpvError::Nomem),
            mpv_error_MPV_ERROR_UNINITIALIZED => Self::MpvKnown(MpvError::Uninitialized),
            mpv_error_MPV_ERROR_INVALID_PARAMETER => Self::MpvKnown(MpvError::InvalidParameter),
            mpv_error_MPV_ERROR_OPTION_NOT_FOUND => Self::MpvKnown(MpvError::OptionNotFound),
            mpv_error_MPV_ERROR_OPTION_FORMAT => Self::MpvKnown(MpvError::OptionFormat),
            mpv_error_MPV_ERROR_OPTION_ERROR => Self::MpvKnown(MpvError::OptionError),
            mpv_error_MPV_ERROR_PROPERTY_NOT_FOUND => Self::MpvKnown(MpvError::PropertyNotFound),
            mpv_error_MPV_ERROR_PROPERTY_FORMAT => Self::MpvKnown(MpvError::PropertyFormat),
            mpv_error_MPV_ERROR_PROPERTY_UNAVAILABLE => Self::MpvKnown(MpvError::PropertyUnavailable),
            mpv_error_MPV_ERROR_PROPERTY_ERROR => Self::MpvKnown(MpvError::PropertyError),
            mpv_error_MPV_ERROR_COMMAND => Self::MpvKnown(MpvError::Command),
            mpv_error_MPV_ERROR_LOADING_FAILED => Self::MpvKnown(MpvError::LoadingFailed),
            mpv_error_MPV_ERROR_AO_INIT_FAILED => Self::MpvKnown(MpvError::AoInitFailed),
            mpv_error_MPV_ERROR_VO_INIT_FAILED => Self::MpvKnown(MpvError::VoInitFailed),
            mpv_error_MPV_ERROR_NOTHING_TO_PLAY => Self::MpvKnown(MpvError::NothingToPlay),
            mpv_error_MPV_ERROR_UNKNOWN_FORMAT => Self::MpvKnown(MpvError::UnknownFormat),
            mpv_error_MPV_ERROR_UNSUPPORTED => Self::MpvKnown(MpvError::Unsupported),
            mpv_error_MPV_ERROR_NOT_IMPLEMENTED => Self::MpvKnown(MpvError::NotImplemented),
            mpv_error_MPV_ERROR_GENERIC => Self::MpvKnown(MpvError::Generic),
            other => Self::MpvUnknown(other),
        }
    }
}
