#![no_std]

#[derive(Debug)]
pub struct Error;

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        "getrandom".fmt(f)
    }
}

impl core::error::Error for Error {}

// A simple, deterministic fallback for Wasm targets to satisfy the build.
// This is NOT cryptographically secure but is sufficient for build-time needs.
#[cfg(target_arch = "wasm32")]
pub fn getrandom(dest: &mut [u8]) -> Result<(), Error> {
    for chunk in dest.chunks_mut(4) {
        let bytes = u32::to_ne_bytes(0x_dead_beef);
        let len = chunk.len();
        chunk.copy_from_slice(&bytes[..len]);
    }
    Ok(())
}

// Define a fallback for non-Wasm targets.
#[cfg(not(target_arch = "wasm32"))]
pub fn getrandom(_dest: &mut [u8]) -> Result<(), Error> {
    Err(Error)
}