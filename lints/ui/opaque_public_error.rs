#![crate_type = "lib"]

// A library crate: only items reachable from outside it are checked.

use std::error::Error;
use std::fmt;

pub fn boxed() -> Result<(), Box<dyn Error>> {
    Ok(())
}

pub fn boxed_send_sync() -> Result<u8, Box<dyn Error + Send + Sync + 'static>> {
    Ok(0)
}

pub fn with_anyhow() -> anyhow::Result<String> {
    Ok(String::new())
}

pub fn anyhow_spelled_out() -> Result<(), anyhow::Error> {
    Ok(())
}

pub async fn async_anyhow() -> anyhow::Result<()> {
    Ok(())
}

pub struct Client;

impl Client {
    pub fn fetch(&self) -> anyhow::Result<Vec<u8>> {
        Ok(Vec::new())
    }
}

pub trait Store {
    fn get(&self, key: &str) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>>;
}

pub mod nested {
    pub fn reexported() -> anyhow::Result<()> {
        Ok(())
    }
}

pub use nested::reexported;

#[derive(Debug)]
pub enum FetchError {
    Timeout,
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("request timed out")
    }
}

impl Error for FetchError {}

// Not linted: a concrete, matchable error type.
pub fn concrete() -> Result<(), FetchError> {
    Err(FetchError::Timeout)
}

// Not linted: private to the crate.
fn private_helper() -> anyhow::Result<()> {
    Ok(())
}

// Not linted: `pub(crate)` is not reachable by other crates.
pub(crate) fn crate_helper() -> anyhow::Result<()> {
    private_helper()
}

mod private {
    // Not linted: `pub` inside a private module that is never re-exported.
    pub fn unreachable_from_outside() -> Result<(), Box<dyn std::error::Error>> {
        super::crate_helper().map_err(Into::into)
    }
}

pub struct Disk;

// Not linted: the trait declaration owns the signature.
impl Store for Disk {
    fn get(&self, _key: &str) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        private::unreachable_from_outside().map_err(|_| "unavailable".into()).map(|()| Vec::new())
    }
}

// Not linted: a boxed concrete error, and an opaque error in the success position.
pub fn boxed_concrete() -> Result<Box<dyn Error>, Box<FetchError>> {
    Ok(Box::new(FetchError::Timeout))
}
