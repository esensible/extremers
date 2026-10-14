//! Bundles over BLE, the host's side (BLE.md "Bundles"; the messages are
//! extreme-protocol's [`BundleRequest`] and [`BundleResponse`]): what a
//! bundle request written to `event` does, what an upload's `data` writes
//! do, and when an idle upload is dropped. [`crate::ble`] moves the bytes:
//! it hands each request to [`Host::ble_event`] and each `data` write to
//! [`Host::bundle_data`], and notifies what they answer.
//!
//! An upload belongs to the connection that began it (a [`BleClient`]),
//! and lives on in the store when that connection drops: a `Begin` of the
//! same bundle (id, length, SHA-256) within [`UPLOAD_IDLE_SECS`] of its
//! last activity resumes it where it is, from any connection, and makes
//! that connection its owner. Another upload is refused (`Busy`) while it
//! is live; once it has been idle that long it is dropped
//! ([`Host::bundle_expire`], which the BLE task runs at
//! [`Host::bundle_deadline`]) and its owner is told `Timeout` at its next
//! request.
//!
//! A firmware update (`BeginFirmware`, BLE.md "Firmware update") is an
//! upload like a bundle's, of a [`Target::Firmware`]: the same session, the
//! same `data` writes and Acks, `Commit` and `Abort` act on it, and it is
//! dropped the same way when idle. While either kind is live the other is
//! refused (`Busy`), whichever connection asks.

use embassy_time::{Duration, Instant};
use extreme_protocol::{
    BundleEntry, BundleError, BundleRequest, BundleResponse, CHOOSE, DataPacket, NOT_ALLOWED_ID,
    NOT_ALLOWED_NO_ENGINE, PackageId, REGISTRY_MAX, REQUEST_DATA, UPLOAD_IDLE_SECS, ack_every,
    data_chunk, window,
};
use sha2::{Digest, Sha256};

use crate::{
    bulk,
    fmt::Disp,
    host::{Host, InstallError, Lease},
    store::{Error as StoreError, Target, Uploading},
};

/// An upload idle this long is dropped.
const IDLE: Duration = Duration::from_secs(UPLOAD_IDLE_SECS);

/// A BLE connection, as bundle requests see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BleClient {
    /// Which connection (its BLE worker): an upload belongs to one.
    pub id: u8,
    /// The `data` chunk its ATT MTU carries ([`data_chunk`]).
    pub chunk: u16,
}

impl BleClient {
    /// Connection `id` on a link with ATT MTU `att_mtu`.
    pub fn new(id: usize, att_mtu: u16) -> Self {
        BleClient { id: id as u8, chunk: data_chunk(att_mtu) }
    }
}

/// Every response a request can give at once: a `List`'s entries and its
/// end.
pub type Responses = heapless::Vec<BundleResponse, { REGISTRY_MAX + 1 }>;

/// What the BLE transport sends for a bundle request.
pub enum BundleAnswer<'h> {
    /// Notified on `bundle`, in order.
    Responses(Responses),
    /// A `Get`'s download.
    Download(Download<'h>),
}

impl BundleAnswer<'_> {
    fn one(response: BundleResponse) -> Self {
        let mut responses = Responses::new();
        // cannot fail: the first of REGISTRY_MAX + 1
        let _ = responses.push(response);
        BundleAnswer::Responses(responses)
    }
}

/// A `Get`: [`sending`](Self::sending) on `bundle`, then every one of
/// [`packets`](Self::packets) on `data`, then [`sent`](Self::sent) on
/// `bundle`. The bundle's bytes are read in place under a
/// [`lease`](Host::lease), held until this is dropped.
pub struct Download<'h> {
    pub id: PackageId,
    pub bytes: &'static [u8],
    pub from: u32,
    pub chunk: u16,
    sha256: [u8; 32],
    _lease: Lease<'h>,
}

impl Download<'_> {
    pub fn sending(&self) -> BundleResponse {
        BundleResponse::Sending { id: self.id, len: self.bytes.len() as u32, sha256: self.sha256, chunk: self.chunk }
    }

    /// The bytes from `from`, `chunk` at a time, each with its offset.
    pub fn packets(&self) -> impl Iterator<Item = DataPacket<'static>> + use<> {
        let (bytes, from, chunk) = (self.bytes, self.from as usize, self.chunk as usize);
        bytes[from..].chunks(chunk).enumerate().map(move |(i, b)| DataPacket { offset: (from + i * chunk) as u32, bytes: b })
    }

    pub fn sent(&self) -> BundleResponse {
        BundleResponse::Sent { id: self.id }
    }
}

/// The upload in progress, as the BLE side keeps it (the store, or the
/// firmware's sink, keeps its bytes).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Session {
    /// The connection that owns it.
    client: u8,
    target: Target,
    len: u32,
    /// Its last request or bytes.
    last: Instant,
    /// When it began or was resumed, and from which offset: for the log.
    started: Instant,
    from: u32,
    /// The last offset acknowledged, and how often to.
    acked: u32,
    ack_every: u32,
    /// A gap was reported (`Sequence`) and not yet filled: one report per
    /// gap.
    gap_told: bool,
    /// Dropped for being idle; its owner is told `Timeout` next.
    expired: bool,
}

impl Session {
    fn live(&self, now: Instant) -> bool {
        !self.expired && now < self.last + IDLE
    }
}

fn error(request: u8, code: BundleError, detail: u32) -> BundleResponse {
    BundleResponse::Error { request, code, detail }
}

/// The response to a store refusal of `request` ([`BundleError`]'s details).
pub fn store_error(request: u8, e: StoreError) -> BundleResponse {
    use bundle::Invalid;
    let invalid = |i: Invalid| match i {
        Invalid::Reason { reason, detail } => (BundleError::Invalid, reason as u32 | detail << 8),
        Invalid::OverLimit { which } => (BundleError::OverLimit, which as u32),
    };
    let (code, detail) = match e {
        StoreError::Busy | StoreError::Leased => (BundleError::Busy, 0),
        StoreError::NoSpace => (BundleError::NoSpace, 0),
        StoreError::NoUpload => (BundleError::NoUpload, 0),
        StoreError::Sequence { expected } => (BundleError::Sequence, expected),
        StoreError::Length => (BundleError::Length, 0),
        StoreError::Hash => (BundleError::Hash, 0),
        StoreError::Invalid(i) | StoreError::Install(InstallError::Invalid(i)) => invalid(i),
        StoreError::Id | StoreError::Install(InstallError::Id) => (BundleError::NotAllowed, NOT_ALLOWED_ID),
        StoreError::Install(InstallError::Abi) => (BundleError::NotAllowed, NOT_ALLOWED_NO_ENGINE),
        StoreError::Install(InstallError::Full) => (BundleError::Full, 0),
        StoreError::NotFound => (BundleError::NotFound, 0),
        StoreError::Flash => (BundleError::Flash, 0),
        // `firmware::reason`, then its detail, as a bundle's
        StoreError::Image { reason, detail } => (BundleError::Invalid, reason as u32 | detail << 8),
    };
    error(request, code, detail)
}

impl Host {
    fn session(&self) -> Option<Session> {
        self.upload.lock(|s| s.get())
    }

    fn set_session(&self, session: Option<Session>) {
        self.upload.lock(|s| s.set(session));
    }

    /// Refuses `request` from `client` if another connection's upload is
    /// live (`Busy`), or tells the owner of one dropped for being idle
    /// (`Timeout`, once).
    fn may_touch_upload(&self, client: BleClient, request: u8) -> Result<(), BundleResponse> {
        let now = Instant::now();
        self.upload.lock(|cell| match cell.get() {
            Some(s) if s.expired && s.client == client.id => {
                cell.set(None);
                Err(error(request, BundleError::Timeout, 0))
            }
            Some(s) if s.live(now) && s.client != client.id => Err(error(request, BundleError::Busy, 0)),
            _ => Ok(()),
        })
    }

    /// A bundle request from `client` (BLE.md "Bundles").
    pub(crate) async fn bundle_request(&self, request: BundleRequest, client: BleClient) -> BundleAnswer<'_> {
        let code = request.code();
        let one = BundleAnswer::one;
        match request {
            BundleRequest::List => BundleAnswer::Responses(self.bundle_list().await),
            BundleRequest::Begin { id, len, sha256 } => one(self.ble_begin(client, Target::Bundle(id), len, &sha256).await),
            BundleRequest::BeginFirmware { len, sha256 } => one(self.ble_begin(client, Target::Firmware, len, &sha256).await),
            BundleRequest::Commit => {
                if let Err(refused) = self.may_touch_upload(client, code) {
                    return one(refused);
                }
                let session = self.session();
                if session.is_some_and(|s| s.target == Target::Firmware) {
                    return one(self.ble_firmware_commit(code, session).await);
                }
                one(match self.bundle_commit().await {
                    Ok(package) => {
                        if let Some(s) = session.filter(|s| matches!(s.target, Target::Bundle(_))) {
                            let ms = s.started.elapsed().as_millis();
                            let sent = (s.len - s.from) as usize;
                            info!(
                                "bundles: {} v{} uploaded and installed: {} B in {} ms ({} B/s)",
                                package.name,
                                package.version,
                                sent,
                                ms,
                                bulk::throughput(sent, ms)
                            );
                        }
                        self.set_session(None);
                        BundleResponse::Committed { id: package.id, content_hash: package.content_hash }
                    }
                    // not every byte is in yet: the upload goes on
                    Err(StoreError::Length) => {
                        let written = self.bundle_uploading().await.map_or(0, |u| u.written);
                        error(code, BundleError::Length, written)
                    }
                    Err(e) => {
                        warn!("bundles: upload refused at its commit: {:?}", crate::fmt::Dbg(&e));
                        self.set_session(None);
                        store_error(code, e)
                    }
                })
            }
            BundleRequest::Abort => {
                if let Err(refused) = self.may_touch_upload(client, code) {
                    return one(refused);
                }
                let session = self.session();
                self.set_session(None);
                let aborted = match session.map(|s| s.target) {
                    Some(Target::Firmware) => self.firmware_abort().await,
                    _ => self.bundle_abort().await,
                };
                one(if aborted { BundleResponse::Aborted } else { error(code, BundleError::NoUpload, 0) })
            }
            BundleRequest::Delete { id } if id == CHOOSE => one(error(code, BundleError::NotAllowed, NOT_ALLOWED_ID)),
            BundleRequest::Delete { id } => one(match self.bundle_delete(id).await {
                Ok(()) => BundleResponse::Deleted { id },
                Err(e) => store_error(code, e),
            }),
            BundleRequest::Activate { id } => one(match self.activate(id).await {
                Ok(()) => BundleResponse::Activated { id },
                Err(_) => error(code, BundleError::NotFound, 0),
            }),
            BundleRequest::Get { id, from } => {
                // taken before the lookup: a bundle found is still
                // installed, or its sectors wait for this read
                let lease = self.lease();
                let Some(package) = self.package(id) else {
                    return one(error(code, BundleError::NotFound, 0));
                };
                let bytes = package.archive.bytes();
                if from as usize > bytes.len() || client.chunk == 0 {
                    return one(error(code, BundleError::Length, bytes.len() as u32));
                }
                let sha256 = Sha256::digest(bytes).into();
                BundleAnswer::Download(Download { id, bytes, from, chunk: client.chunk, sha256, _lease: lease })
            }
            BundleRequest::Firmware => one(self.firmware_info().unwrap_or(error(code, BundleError::NotFound, 0))),
        }
    }

    /// `Commit` of a firmware update: `Rebooting` once the image checked
    /// out and the next boot runs it (the target restarts shortly after);
    /// why not, otherwise (then the target restarts into the running
    /// firmware), but for an image not all in yet, which goes on.
    async fn ble_firmware_commit(&self, code: u8, session: Option<Session>) -> BundleResponse {
        match self.firmware_commit().await {
            Ok(_) => {
                if let Some(s) = session {
                    let ms = s.started.elapsed().as_millis();
                    let sent = (s.len - s.from) as usize;
                    info!("bundles: firmware uploaded and checked: {} B in {} ms ({} B/s); rebooting", sent, ms, bulk::throughput(sent, ms));
                }
                self.set_session(None);
                BundleResponse::Rebooting
            }
            Err(StoreError::Length) => {
                let written = self.firmware_uploading().await.map_or(0, |u| u.written);
                error(code, BundleError::Length, written)
            }
            Err(e) => {
                warn!("bundles: firmware refused at its commit: {:?}", crate::fmt::Dbg(&e));
                self.set_session(None);
                store_error(code, e)
            }
        }
    }

    /// The upload in progress, of either kind.
    async fn uploading(&self) -> Option<Uploading> {
        match self.firmware_uploading().await {
            Some(u) => Some(u),
            None => self.bundle_uploading().await,
        }
    }

    /// `List`: an entry per package, Choose first, then the free space.
    async fn bundle_list(&self) -> Responses {
        let active = self.current_state().await.active;
        let (packages, boot) = self.registry.lock(|r| {
            let r = r.borrow();
            (r.all(), r.boot_package().map(|p| p.id))
        });
        let mut out = Responses::new();
        for p in &packages {
            let entry = BundleEntry {
                manifest: p.manifest(),
                len: p.archive.bytes().len() as u32,
                active: p.id == active,
                boot: boot == Some(p.id),
                builtin: p.id == CHOOSE,
            };
            // cannot fail: at most REGISTRY_MAX packages
            let _ = out.push(BundleResponse::Entry(entry));
        }
        let space = self.bundle_space().await;
        let _ = out.push(BundleResponse::ListEnd {
            count: packages.len() as u8,
            free_bytes: space.map_or(0, |s| s.free_bytes),
            largest_free: space.map_or(0, |s| s.largest_free),
        });
        out
    }

    /// `Begin` (or `BeginFirmware`): a new upload of `target`, or the one in
    /// progress resumed.
    async fn ble_begin(&self, client: BleClient, target: Target, len: u32, sha256: &[u8; 32]) -> BundleResponse {
        let code = match target {
            Target::Bundle(_) => 1,
            Target::Firmware => 8,
        };
        if client.chunk == 0 {
            return error(code, BundleError::Length, 0);
        }
        let now = Instant::now();
        let w = window(client.chunk);
        let ready = |offset| BundleResponse::Ready { offset, chunk: client.chunk, window: w };
        let session = |from| Session {
            client: client.id,
            target,
            len,
            last: now,
            started: now,
            from,
            acked: from,
            ack_every: ack_every(w),
            gap_told: false,
            expired: false,
        };
        if let Some(u) = self.uploading().await {
            let live = self.session().is_some_and(|s| s.live(now));
            if live && (u.target, u.len, &u.sha256) == (target, len, sha256) {
                info!("bundles: upload of {} resumed by connection {} at {} of {} B", Disp(&target), client.id, u.written, len);
                self.set_session(Some(session(u.written)));
                return ready(u.written);
            }
            let same_kind = matches!((u.target, target), (Target::Bundle(_), Target::Bundle(_)) | (Target::Firmware, Target::Firmware));
            if live && (!same_kind || self.session().is_some_and(|s| s.client != client.id)) {
                return error(code, BundleError::Busy, 0);
            }
            // this connection's own, or idle: it makes way for the new one
            // (a firmware image's is started afresh by its begin)
            info!("bundles: upload of {} dropped for a new one", Disp(&u.target));
            if let Target::Bundle(_) = u.target {
                self.bundle_abort().await;
            }
            self.set_session(None);
        }
        let begun = match target {
            Target::Bundle(id) => self.bundle_begin(id, len, sha256).await,
            Target::Firmware => self.firmware_begin(len, sha256).await,
        };
        match begun {
            Ok(()) => {
                self.set_session(Some(session(0)));
                ready(0)
            }
            Err(e) => {
                // an old upload's leftovers go; another connection's upload
                // that began meanwhile (this one then got `Busy`) stays
                let now = Instant::now();
                self.upload.lock(|s| {
                    if s.get().is_some_and(|s| !s.live(now)) {
                        s.set(None);
                    }
                });
                store_error(code, e)
            }
        }
    }

    /// A write to `data` from `client` (`offset u32 LE` then the bytes):
    /// the upload's next bytes. Answered (on `bundle`) with an `Ack` every
    /// so often and at the end, a `Sequence` error once per gap, or why the
    /// bytes were refused; bytes it already has (a client going back to the
    /// offset a `Sequence` named) are ignored.
    pub async fn bundle_data(&self, client: BleClient, value: &[u8]) -> Option<BundleResponse> {
        let refused = |code| Some(error(REQUEST_DATA, code, 0));
        let Some(packet) = DataPacket::parse(value) else {
            return refused(BundleError::Length);
        };
        let target = match self.session() {
            None => return refused(BundleError::NoUpload),
            Some(s) if s.expired && s.client == client.id => {
                self.set_session(None);
                return refused(BundleError::Timeout);
            }
            Some(s) if s.expired => return refused(BundleError::NoUpload),
            Some(s) if s.client != client.id => return refused(BundleError::Busy),
            Some(s) => s.target,
        };
        let written = match target {
            Target::Bundle(_) => self.bundle_write(packet.offset, packet.bytes).await,
            Target::Firmware => self.firmware_write(packet.offset, packet.bytes).await,
        };
        // the session as it is now: the write may have waited for the store
        self.upload.lock(|cell| {
            let mut s = cell.get().filter(|s| s.client == client.id && !s.expired)?;
            s.last = Instant::now();
            let response = match written {
                Ok(written) => {
                    s.gap_told = false;
                    if written - s.acked >= s.ack_every || written == s.len {
                        s.acked = written;
                        Some(BundleResponse::Ack { offset: written })
                    } else {
                        None
                    }
                }
                // bytes it has: a repeat after a rewind
                Err(StoreError::Sequence { expected }) if packet.offset < expected => None,
                Err(StoreError::Sequence { expected }) => {
                    if s.gap_told {
                        None
                    } else {
                        s.gap_told = true;
                        debug!("bundles: gap at {}: expected {}", packet.offset, expected);
                        Some(error(REQUEST_DATA, BundleError::Sequence, expected))
                    }
                }
                Err(e) => Some(store_error(REQUEST_DATA, e)),
            };
            cell.set(Some(s));
            response
        })
    }

    /// The connection whose upload is live, if any: the BLE side keeps that
    /// link at a short connection interval meanwhile (BLE.md "Throughput").
    pub fn bundle_uploader(&self) -> Option<u8> {
        let now = Instant::now();
        self.session().filter(|s| s.live(now)).map(|s| s.client)
    }

    /// When the upload in progress is dropped unless it hears from its
    /// client before; `None` without one.
    pub fn bundle_deadline(&self) -> Option<Instant> {
        self.session().filter(|s| !s.expired).map(|s| s.last + IDLE)
    }

    /// Drops the upload in progress if it has been idle for
    /// [`UPLOAD_IDLE_SECS`] at `now`; whether it was.
    pub async fn bundle_expire(&self, now: Instant) -> bool {
        let Some(s) = self.session().filter(|s| !s.expired && now >= s.last + IDLE) else {
            return false;
        };
        match s.target {
            Target::Bundle(_) => self.bundle_abort().await,
            Target::Firmware => self.firmware_abort().await,
        };
        self.set_session(Some(Session { expired: true, ..s }));
        warn!(
            "bundles: upload of {} idle for {} s: dropped ({} of {} B acknowledged)",
            Disp(&s.target),
            UPLOAD_IDLE_SECS,
            s.acked,
            s.len
        );
        true
    }
}
