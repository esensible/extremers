//! A firmware update's image, written into the data slot (on the C6 the
//! inactive app slot, which the update then boots): [`Firmware`] is what the
//! host writes through, [`FirmwareSink`] the one way to do it over NOR flash
//! read in place ([`TrackFlash`]).
//!
//! The image comes in order, as a bundle upload does ([`program`]: each
//! sector erased just before it is first written, unless it is blank), and
//! is hashed as it goes, all but its last 32 bytes: those are the image's
//! appended digest, which [`Firmware::begin`] was told. [`Firmware::finish`]
//! checks the hash, that the digest is in flash, and the image's structure
//! (`extreme_protocol::firmware::check`) as the flash holds it; only then
//! may the target boot it.

use core::ptr;

use extreme_protocol::firmware::{self, DIGEST_LEN};
use sha2::{Digest, Sha256};

use crate::{
    store::{Error, Target, Uploading, program},
    track::{SECTOR_LEN, TrackFlash},
};

/// The most sectors an image takes: a 2 MiB slot.
pub const MAX_SECTORS: usize = 512;

/// A firmware image's upload, as the host uses it (object safe: the host
/// holds a `&'static mut dyn Firmware`, whatever flash is under it).
pub trait Firmware {
    /// Starts an upload of an image of `len` bytes whose appended digest is
    /// `digest`, replacing one in progress; writes nothing yet.
    /// [`Error::NoSpace`] if it is longer than [`max_len`](Self::max_len),
    /// [`Error::Length`] if it can't be an image (no room for a digest).
    fn begin(&mut self, len: u32, digest: &[u8; 32]) -> Result<(), Error>;
    /// Writes the image's next bytes, from `offset` (which must be where it
    /// is up to: [`Error::Sequence`] otherwise); every chunk but the last a
    /// multiple of 4 bytes. The bytes written so far.
    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<u32, Error>;
    /// Ends the upload, once every byte is written ([`Error::Length`]
    /// before, and the upload goes on): the hash of all but the last 32
    /// bytes is the digest ([`Error::Hash`]), the digest reads back from
    /// flash, and the image checks out there ([`Error::Image`]). Its digest.
    /// Every outcome but [`Error::NoUpload`] and [`Error::Length`] ends the
    /// upload.
    fn finish(&mut self) -> Result<[u8; 32], Error>;
    /// Drops the upload in progress; whether there was one.
    fn abort(&mut self) -> bool;
    fn uploading(&self) -> Option<Uploading>;
    /// The longest image the region holds.
    fn max_len(&self) -> u32;
}

/// The upload in progress.
struct Upload {
    len: u32,
    digest: [u8; 32],
    written: u32,
    /// Sectors ready to be written (erased or blank), from the first.
    ready: u16,
    /// Fed the image's bytes up to its digest.
    hasher: Sha256,
}

/// Firmware images written into `flash` (the whole data slot), from its
/// start.
pub struct FirmwareSink<F: TrackFlash> {
    flash: F,
    upload: Option<Upload>,
}

impl<F: TrackFlash> FirmwareSink<F> {
    pub fn new(flash: F) -> Self {
        assert!(flash.bytes().len() <= MAX_SECTORS * SECTOR_LEN, "a firmware region is at most {MAX_SECTORS} sectors");
        FirmwareSink { flash, upload: None }
    }
}

impl<F: TrackFlash> Firmware for FirmwareSink<F> {
    fn begin(&mut self, len: u32, digest: &[u8; 32]) -> Result<(), Error> {
        if len <= DIGEST_LEN {
            return Err(Error::Length);
        }
        if len > self.max_len() {
            return Err(Error::NoSpace);
        }
        self.upload = Some(Upload { len, digest: *digest, written: 0, ready: 0, hasher: Sha256::new() });
        Ok(())
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<u32, Error> {
        let u = self.upload.as_mut().ok_or(Error::NoUpload)?;
        if offset != u.written {
            return Err(Error::Sequence { expected: u.written });
        }
        let end = offset.checked_add(bytes.len() as u32).ok_or(Error::Length)?;
        if end > u.len || (bytes.len() % 4 != 0 && end != u.len) {
            return Err(Error::Length);
        }
        program(&mut self.flash, &mut u.ready, 0, offset, bytes)?;
        // the digest is not part of what it hashes
        let hashed = (u.len - DIGEST_LEN).saturating_sub(offset).min(bytes.len() as u32) as usize;
        u.hasher.update(&bytes[..hashed]);
        u.written = end;
        Ok(end)
    }

    fn finish(&mut self) -> Result<[u8; 32], Error> {
        let u = self.upload.as_ref().ok_or(Error::NoUpload)?;
        if u.written != u.len {
            return Err(Error::Length);
        }
        let Some(u) = self.upload.take() else {
            return Err(Error::NoUpload);
        };
        let hash: [u8; 32] = u.hasher.finalize().into();
        if hash != u.digest {
            return Err(Error::Hash);
        }
        let bytes = self.flash.bytes();
        let at = (u.len - DIGEST_LEN) as usize;
        // SAFETY: within the region (`begin` bounds `len`); flash read in
        // place, written behind the reference, hence volatile.
        let stored = unsafe { ptr::read_volatile(bytes.as_ptr().add(at).cast::<[u8; 32]>()) };
        if stored != u.digest {
            error!("firmware: the digest does not read back from flash");
            return Err(Error::Flash);
        }
        // `check` reads words (offsets and lengths multiples of 4) within `len`
        let read = |at: u32, buf: &mut [u8]| {
            for (i, w) in buf.chunks_exact_mut(4).enumerate() {
                // SAFETY: in the region (above); volatile as above
                w.copy_from_slice(&unsafe { ptr::read_volatile(bytes.as_ptr().add(at as usize + 4 * i).cast::<[u8; 4]>()) });
            }
            Ok::<(), ()>(())
        };
        firmware::check(u.len, read).map_err(|(reason, detail)| Error::Image { reason, detail })?;
        Ok(u.digest)
    }

    fn abort(&mut self) -> bool {
        self.upload.take().is_some()
    }

    fn uploading(&self) -> Option<Uploading> {
        self.upload.as_ref().map(|u| Uploading { target: Target::Firmware, len: u.len, sha256: u.digest, written: u.written })
    }

    fn max_len(&self) -> u32 {
        self.flash.bytes().len() as u32
    }
}

#[cfg(test)]
pub(crate) mod tests;
