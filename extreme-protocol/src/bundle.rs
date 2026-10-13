//! Bundles over BLE (BLE.md "Bundles"): listing, installing, replacing,
//! deleting, activating and downloading the bundles a host holds. A
//! request is a [`HostEvent::Bundle`](crate::HostEvent::Bundle) written to
//! `event`; every answer is a [`BundleResponse`] notified on `bundle`; an
//! upload's bytes are written to `data` and a download's bytes notified on
//! it, each as a [`DataPacket`] (`offset u32 LE` then the bytes). Over BLE
//! only: the websocket refuses them.
//!
//! An upload: `Begin` → `Ready{offset, chunk, window}` → the archive from
//! `offset` in `chunk`s on `data`, at most `window` bytes past the last
//! `Ack` → `Ack{len}` → `Commit` → `Committed`. A `Begin` of the same
//! upload (id, length, SHA-256) within [`UPLOAD_IDLE_SECS`] of its last
//! activity resumes it, from any connection.

use postcard_schema::Schema;
use serde::{Deserialize, Serialize};

use crate::{Manifest, PackageId};

/// Client to host: a [`HostEvent::Bundle`](crate::HostEvent::Bundle)'s
/// request. Each is answered on `bundle`: what it gets, or
/// [`BundleResponse::Error`] with the request's [`code`](Self::code).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub enum BundleRequest {
    /// Every bundle the host runs: an [`BundleResponse::Entry`] each (Choose
    /// first, `builtin`), then [`BundleResponse::ListEnd`].
    List,
    /// Starts (or resumes) an upload of the bundle `id` ([`package_id`] of
    /// its name), `len` bytes whose SHA-256 is `sha256`: answered
    /// [`BundleResponse::Ready`].
    ///
    /// [`package_id`]: crate::package_id
    Begin { id: PackageId, len: u32, sha256: [u8; 32] },
    /// Ends the upload once every byte is in: the bundle is checked (hash,
    /// validation, the manifest's hashes) and installed, replacing the one
    /// with its id; if that was active it restarts, keeping its state when
    /// its interface is unchanged. [`BundleResponse::Committed`].
    Commit,
    /// Drops the upload in progress: [`BundleResponse::Aborted`].
    Abort,
    /// Deletes the bundle `id`; if it was active, back to Choose.
    /// [`BundleResponse::Deleted`].
    Delete { id: PackageId },
    /// Makes `id` active and the one to boot with (Choose is 0).
    /// [`BundleResponse::Activated`].
    Activate { id: PackageId },
    /// Downloads the bundle `id`'s archive from byte `from`:
    /// [`BundleResponse::Sending`], the bytes on `data`, then
    /// [`BundleResponse::Sent`].
    Get { id: PackageId, from: u32 },
}

impl BundleRequest {
    /// The request's variant index, as [`BundleResponse::Error`] names it.
    pub fn code(&self) -> u8 {
        match self {
            BundleRequest::List => 0,
            BundleRequest::Begin { .. } => 1,
            BundleRequest::Commit => 2,
            BundleRequest::Abort => 3,
            BundleRequest::Delete { .. } => 4,
            BundleRequest::Activate { .. } => 5,
            BundleRequest::Get { .. } => 6,
        }
    }
}

/// [`BundleResponse::Error`]'s `request` when the error is about a write to
/// `data` rather than a request.
pub const REQUEST_DATA: u8 = 0xFF;

/// Host to client, on `bundle`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub enum BundleResponse {
    /// One bundle of a `List`.
    Entry(BundleEntry),
    /// The end of a `List`: how many entries came, and the store's free
    /// space (all 0 on a host without a store).
    ListEnd { count: u8, free_bytes: u32, largest_free: u32 },
    /// The upload is ready for bytes from `offset` (0, or where a resumed
    /// one is up to): `data` writes of at most `chunk` bytes (each a
    /// multiple of 4 but the last), at most `window` bytes past the last
    /// `Ack` (or `offset`).
    Ready { offset: u32, chunk: u16, window: u16 },
    /// Every byte before `offset` is written.
    Ack { offset: u32 },
    /// The upload is installed: its id and content hash.
    Committed { id: PackageId, content_hash: [u8; 8] },
    Aborted,
    Deleted { id: PackageId },
    Activated { id: PackageId },
    /// A `Get`'s bytes follow on `data`, `chunk` bytes per notification
    /// (the last shorter): the archive is `len` bytes with SHA-256
    /// `sha256`, sent from the `Get`'s `from`.
    Sending { id: PackageId, len: u32, sha256: [u8; 32], chunk: u16 },
    /// A `Get`'s last bytes went.
    Sent { id: PackageId },
    /// The request ([`BundleRequest::code`], or [`REQUEST_DATA`]) was
    /// refused; `detail` per [`BundleError`].
    Error { request: u8, code: BundleError, detail: u32 },
}

/// A bundle as `List` gives it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct BundleEntry {
    pub manifest: Manifest,
    /// The archive's length, bytes.
    pub len: u32,
    /// It is the active package.
    pub active: bool,
    /// It is the package the host boots with.
    pub boot: bool,
    /// It is part of the firmware (Choose), not in the store: it can be
    /// activated and downloaded, not deleted or replaced.
    pub builtin: bool,
}

/// Why a request was refused, and what [`BundleResponse::Error`]'s
/// `detail` then says (0 where nothing is listed).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub enum BundleError {
    /// Another connection's upload is in progress (and not idle), or a
    /// deleted bundle's sectors are still being read.
    Busy,
    /// No upload is in progress.
    NoUpload,
    /// Not the next bytes of the upload: `detail` is the offset written up
    /// to; send from there. Told once per gap.
    Sequence,
    /// A length that doesn't fit: zero, past the declared length, a chunk
    /// other than the last that isn't a multiple of 4, a `Commit` before
    /// every byte is in (the upload goes on), a `Get` from past the end, a
    /// `data` write without its offset. `detail`: the bytes written so far
    /// (a `Commit`), the archive's length (a `Get`).
    Length,
    /// The bytes' SHA-256 isn't the one `Begin` declared.
    Hash,
    /// The bundle doesn't validate: `detail` is the reason
    /// (`extreme_bundle::validate::reason`) in its low 8 bits and the
    /// reason's own detail's low 24 bits above them.
    Invalid,
    /// The engine needs more than the host has: `detail` is which limit
    /// (`extreme_bundle::validate::limit`).
    OverLimit,
    /// No run of free sectors holds it.
    NoSpace,
    /// The host holds as many bundles as it can: delete one first.
    Full,
    /// No bundle with that id.
    NotFound,
    /// Not for this id: `detail` [`NOT_ALLOWED_ID`] or
    /// [`NOT_ALLOWED_NO_ENGINE`].
    NotAllowed,
    /// A flash write or erase failed (the device logs it).
    Flash,
    /// The upload was idle for [`UPLOAD_IDLE_SECS`] and was dropped: begin
    /// again.
    Timeout,
}

/// [`BundleError::NotAllowed`]'s detail: Choose's id (built in: not
/// uploaded, replaced or deleted), an id that isn't the hash of the
/// manifest's name, or a manifest whose id isn't the one `Begin` named.
pub const NOT_ALLOWED_ID: u32 = 0;
/// [`BundleError::NotAllowed`]'s detail: a bundle without an engine (only
/// Choose is a page alone).
pub const NOT_ALLOWED_NO_ENGINE: u32 = 1;

/// An upload with no request and no bytes for this long is dropped.
pub const UPLOAD_IDLE_SECS: u64 = 60;

/// Most bytes a client keeps unacknowledged ([`BundleResponse::Ready`]'s
/// `window` is this or less: no more than [`WINDOW_CHUNKS`] chunks).
pub const WINDOW_MAX: u16 = 2048;
/// Most chunks a window holds: the writes a host can have queued for one
/// connection (trouble-host's L2CAP receive queue, 8) without dropping one.
pub const WINDOW_CHUNKS: u16 = 8;
/// The host acknowledges at least every this many bytes (and at most every
/// half window).
pub const ACK_EVERY_MAX: u32 = 1024;

/// A `data` value's offset field, bytes.
pub const DATA_HEADER: usize = 4;

/// The bytes a `data` value carries on a link with ATT MTU `att_mtu`: what
/// a write or notification holds (MTU − 3) less the offset, rounded down
/// to a multiple of 4 (240 at the host's MTU of 247).
pub const fn data_chunk(att_mtu: u16) -> u16 {
    let room = (att_mtu as usize).saturating_sub(3 + DATA_HEADER);
    (room / 4 * 4) as u16
}

/// The window for a link whose `data` chunk is `chunk` bytes.
pub const fn window(chunk: u16) -> u16 {
    let w = chunk as u32 * WINDOW_CHUNKS as u32;
    if w > WINDOW_MAX as u32 { WINDOW_MAX } else { w as u16 }
}

/// How often the host acknowledges on a link with `window`: every
/// [`ACK_EVERY_MAX`] bytes or half a window, whichever is less, so a client
/// that fills the window always gets an `Ack` before it has to stop.
pub const fn ack_every(window: u16) -> u32 {
    let half = window as u32 / 2;
    if half < ACK_EVERY_MAX { if half == 0 { 1 } else { half } } else { ACK_EVERY_MAX }
}

/// A `data` value: `offset u32 LE` then the bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataPacket<'a> {
    pub offset: u32,
    pub bytes: &'a [u8],
}

impl<'a> DataPacket<'a> {
    /// `None` without a whole offset.
    pub fn parse(value: &'a [u8]) -> Option<Self> {
        let (offset, bytes) = value.split_first_chunk::<DATA_HEADER>()?;
        Some(DataPacket { offset: u32::from_le_bytes(*offset), bytes })
    }

    /// Writes the packet into `out`; its length (`None` if `out` is too
    /// short).
    pub fn write(&self, out: &mut [u8]) -> Option<usize> {
        let n = DATA_HEADER + self.bytes.len();
        let out = out.get_mut(..n)?;
        out[..DATA_HEADER].copy_from_slice(&self.offset.to_le_bytes());
        out[DATA_HEADER..].copy_from_slice(self.bytes);
        Some(n)
    }
}

/// Largest [`BundleResponse`]: an `Entry` with the longest manifest (tag,
/// a 5-byte id, a 16-byte name and its length, two 3-byte u16s, the three
/// hashes, a 5-byte `watch_len`), a 5-byte `len` and the three flags.
pub const BUNDLE_RESPONSE_MAX: usize = 1 + (5 + 1 + crate::NAME_MAX + 3 + 3 + 8 + 16 + 5) + 5 + 3;

/// Largest [`HostEvent::Bundle`](crate::HostEvent::Bundle): its tag, then a
/// `Begin` (tag, two 5-byte varints, the SHA-256).
pub const BUNDLE_EVENT_MAX: usize = 1 + 1 + 5 + 5 + 32;
