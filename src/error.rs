//! SDK-wide error model.
//!
//! The Python SDK exposes several exception subclasses; Rust collapses those
//! into one enum so callers can use idiomatic `Result<T, PhantasmaError>` while
//! still distinguishing encoding, serialization, crypto, builder, RPC, HTTP,
//! and JSON failures.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, PhantasmaError>;

#[derive(Debug, Clone, Error)]
pub enum PhantasmaError {
    #[error("encoding error: {0}")]
    Encoding(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("builder error: {0}")]
    Builder(String),
    #[error("rpc error: {message}")]
    Rpc { code: Option<i64>, message: String },
    #[error("http error: {0}")]
    Http(String),
    #[error("json error: {0}")]
    Json(String),
}

impl From<hex::FromHexError> for PhantasmaError {
    fn from(value: hex::FromHexError) -> Self {
        Self::Encoding(value.to_string())
    }
}

impl From<serde_json::Error> for PhantasmaError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value.to_string())
    }
}

impl From<reqwest::Error> for PhantasmaError {
    fn from(value: reqwest::Error) -> Self {
        Self::Http(value.to_string())
    }
}

pub(crate) fn encoding<T>(message: impl Into<String>) -> Result<T> {
    Err(PhantasmaError::Encoding(message.into()))
}

pub(crate) fn serialization<T>(message: impl Into<String>) -> Result<T> {
    Err(PhantasmaError::Serialization(message.into()))
}

pub(crate) fn crypto<T>(message: impl Into<String>) -> Result<T> {
    Err(PhantasmaError::Crypto(message.into()))
}

pub(crate) fn builder<T>(message: impl Into<String>) -> Result<T> {
    Err(PhantasmaError::Builder(message.into()))
}

pub(crate) fn rpc<T>(code: Option<i64>, message: impl Into<String>) -> Result<T> {
    Err(PhantasmaError::Rpc {
        code,
        message: message.into(),
    })
}
