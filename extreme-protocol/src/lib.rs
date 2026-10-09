//! The host protocol (README "Host protocol", BLE.md): what the package host
//! on the C6 and its clients (the Kindle pages, the watch) say to each
//! other, the same on the websocket and over BLE, plus a package's
//! manifest. Everything is [postcard](https://postcard.jamesmunns.com/wire-format);
//! `tools/bindgen` generates the clients' JS and Swift from these types'
//! schemas (postcard-schema's `Schema`).
//!
//! The host owns the selection: the *active* package. It sends every client
//! an [`Envelope`] on every change (and as a heartbeat on the websocket):
//! the device's clock, which package is active, the hashes of that
//! package's UIs, and the package's view as opaque bytes, which the client decodes
//! with that package's own generated bindings. Clients send a
//! [`HostEvent`]: select a package, go back, or an event for the active
//! package's engine (again opaque bytes, encoded with the package's
//! bindings).

#![no_std]

mod package;
pub use package::Package;

use postcard_schema::Schema;
use serde::{Deserialize, Serialize};

/// A package's identity: [`package_id`] of its name, stable across
/// versions. [`CHOOSE`] is the built-in chooser.
pub type PackageId = u32;

/// The built-in Choose package: the host's own list of the installed
/// packages, and the pairing settings on the websocket.
pub const CHOOSE: PackageId = 0;

/// Longest package name, bytes.
pub const NAME_MAX: usize = 16;

/// A package name.
pub type Name = heapless::String<NAME_MAX>;

/// Packages a host can hold at once, Choose included.
pub const REGISTRY_MAX: usize = 8;

/// Most bonded BLE devices a host keeps (BLE.md "Security").
pub const MAX_BLE_BONDS: usize = 4;

/// Largest view in an [`Envelope`] (`state`), bytes: an engine's view
/// (`extreme_sdk::abi::IO_CAP`) or a [`ChooseView`].
pub const STATE_MAX: usize = 256;

/// Largest [`Envelope`]: a 10-byte timestamp, a 5-byte id, the two UI
/// hashes, a 2-byte length and the state.
pub const ENVELOPE_MAX: usize = 10 + 5 + 8 + 8 + 2 + STATE_MAX;

/// Largest engine event in a [`HostEvent::Engine`], bytes.
pub const EVENT_MAX: usize = 256;

/// Largest [`HostEvent`]: tag, id, length, event.
pub const HOST_EVENT_MAX: usize = 1 + 5 + 2 + EVENT_MAX;

/// FNV-1a (32-bit) of `name`: a package's [`PackageId`].
pub const fn package_id(name: &str) -> PackageId {
    let bytes = name.as_bytes();
    let mut hash: u32 = 0x811c_9dc5;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u32;
        hash = hash.wrapping_mul(0x0100_0193);
        i += 1;
    }
    hash
}

/// A package's manifest (`manifest.bin` in the package; the host keeps it
/// in its registry).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Manifest {
    /// [`package_id`] of `name`.
    pub id: PackageId,
    pub name: Name,
    pub version: u16,
    /// The engine ABI the package's engine.wasm speaks
    /// (`extreme_sdk::abi::VERSION`).
    pub abi: u16,
    /// The first 8 bytes of the SHA-256 of engine.wasm, the web bundle and
    /// the watch bundle, in that order (README "Packages"): changes
    /// whenever any of them does.
    pub content_hash: [u8; 8],
    /// Each client's UI on its own: the envelope's `ui`.
    pub ui: UiHashes,
    /// The watch bundle's length, bytes (0 without one): what a `GetAsset`
    /// for it streams (BLE.md "Watch bundles").
    pub watch_len: u32,
}

/// Each client's UI of a package, by content: the first 8 bytes of the
/// SHA-256 of what that client loads (README "Packages"). A client reloads
/// only when its own one changes, so a change to the watch bundle does not
/// reload the Kindles and the other way round.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct UiHashes {
    /// The Kindle page: its web files, in name order, each as name, NUL,
    /// length (u32 LE) and contents.
    pub web: [u8; 8],
    /// The watch bundle (`watch.js`), as is; the hash of no bytes for a
    /// package without one.
    pub watch: [u8; 8],
}

/// Host to client, on every change of the active package or its view: the
/// websocket's binary frames, BLE's `state` characteristic.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Schema)]
pub struct Envelope<'a> {
    /// The device's clock when this was sent, epoch ms (uptime until the
    /// GPS has given it the time). Clients keep `offset = timestamp − their
    /// clock` from every envelope and stamp user actions with their clock
    /// plus it.
    pub timestamp: u64,
    /// The active package.
    pub active: PackageId,
    /// The active package's UIs: a client showing another UI than its own
    /// one here (another package, or another build of it) loads it. The
    /// Kindle pages watch `web`, the watch `watch`.
    pub ui: UiHashes,
    /// The active package's view, postcard: decoded with the package's own
    /// bindings ([`ChooseView`] for Choose).
    pub state: &'a [u8],
}

/// BLE bulk channel (BLE.md "Watch bundles"), client to host: one request
/// per asset, on an L2CAP channel. `hash` is the asset's hash as the client
/// last saw it in an envelope (`ui.watch`); the host answers with an
/// [`AssetHeader`] and the asset's bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct GetAsset {
    pub package: PackageId,
    pub hash: [u8; 8],
}

/// Largest [`GetAsset`]: a 5-byte id and the hash.
pub const GET_ASSET_MAX: usize = 5 + 8;

/// BLE bulk channel, host to client: the answer to a [`GetAsset`], followed
/// by exactly `len` bytes of the asset (none unless `status` is
/// [`ASSET_OK`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct AssetHeader {
    /// [`ASSET_OK`], [`ASSET_UNKNOWN_PACKAGE`] or [`ASSET_HASH_NOT_CURRENT`].
    pub status: u8,
    pub len: u32,
}

/// Largest [`AssetHeader`]: the status and a 5-byte length.
pub const ASSET_HEADER_MAX: usize = 1 + 5;

/// [`AssetHeader::status`]: here it comes.
pub const ASSET_OK: u8 = 0;
/// [`AssetHeader::status`]: no such package is installed.
pub const ASSET_UNKNOWN_PACKAGE: u8 = 1;
/// [`AssetHeader::status`]: the package's watch bundle is not (any more)
/// the one with that hash.
pub const ASSET_HASH_NOT_CURRENT: u8 = 2;

/// Choose's view: the installed packages to choose from (Choose itself not
/// among them), and the BLE pairing status on the websocket (`None` over
/// BLE, and on a host without BLE).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Schema)]
pub struct ChooseView {
    pub packages: heapless::Vec<PackageInfo, REGISTRY_MAX>,
    pub ble: Option<BleStatusView>,
}

/// An installed package, as Choose lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Schema)]
pub struct PackageInfo {
    pub id: PackageId,
    pub name: Name,
    pub version: u16,
}

/// Client to host: the websocket's binary frames, BLE's `event`
/// characteristic.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Schema)]
pub enum HostEvent<'a> {
    /// Make this package active (a fresh engine). Refused for a package
    /// that is not installed; the active one again changes nothing.
    Select(PackageId),
    /// Back: Choose, or the only app when Choose is not installed.
    SelectDefault,
    /// An event for the active package's engine (postcard, its own
    /// `Event`). Refused unless `package` is the active package and its
    /// engine takes the bytes.
    Engine { package: PackageId, bytes: &'a [u8] },
    /// The Kindle's pairing settings: the websocket only.
    Ble(BleCommand),
}

/// What the Kindle's settings panel asks of the BLE transport (BLE.md
/// "Pairing a watch").
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Schema)]
pub enum BleCommand {
    /// Open the pairing window.
    Pair,
    /// Close it early.
    Cancel,
    /// Delete every bond (flash and host) and drop bonded links.
    ForgetAll,
}

/// The last thing that happened to pairing, for the settings panel, which
/// puts it in words. Addresses are in BLE byte order.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize, Schema)]
pub enum BleNote {
    #[default]
    None,
    /// A device bonded (its identity address).
    Paired([u8; 6]),
    /// A device bonded but the bond could not be written to flash: it works
    /// until the next power cycle only.
    PairedNotSaved([u8; 6]),
    /// Pairing was attempted outside the window and refused.
    Rejected,
    /// Pairing started in the window but did not complete.
    Failed,
    /// The window closed with nobody paired.
    Expired,
    /// A device tried to resume a bond this box does not hold.
    StaleBond,
    /// All bonds were deleted.
    Forgotten,
}

/// The BLE transport's state as the Kindle's settings panel shows it
/// ([`ChooseView::ble`]). Addresses are in BLE byte order (least
/// significant byte first): print them reversed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Schema)]
pub struct BleStatusView {
    /// This box's address.
    pub addr: [u8; 6],
    /// What is left of the pairing window when the message was sent, ms; 0
    /// when it is closed.
    pub pair_ms: u32,
    /// The bonded devices' identity addresses, oldest first.
    pub bonds: heapless::Vec<[u8; 6], MAX_BLE_BONDS>,
    /// How many bonds the box keeps.
    pub max_bonds: u8,
    pub note: BleNote,
}

/// Largest serialized `Option<BleStatusView>`: Some, the address, a 5-byte
/// `pair_ms`, the bond count and every bond, `max_bonds`, the longest note.
pub const BLE_STATUS_VIEW_MAX: usize = 1 + 6 + 5 + 1 + 6 * MAX_BLE_BONDS + 1 + 1 + 6;

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    #[test]
    fn ids() {
        assert_eq!(package_id(""), 0x811c_9dc5);
        // FNV-1a test vectors
        assert_eq!(package_id("a"), 0xe40c_292c);
        assert_eq!(package_id("foobar"), 0xbf9c_f968);
        assert_ne!(package_id("race"), CHOOSE);
        assert_ne!(package_id("tune"), CHOOSE);
    }

    #[test]
    fn envelope_on_the_wire() {
        let e = Envelope {
            timestamp: 7,
            active: 300,
            ui: UiHashes {
                web: [1, 2, 3, 4, 5, 6, 7, 8],
                watch: [11, 12, 13, 14, 15, 16, 17, 18],
            },
            state: &[9, 10],
        };
        let bytes = postcard::to_stdvec(&e).unwrap();
        assert_eq!(
            bytes,
            [7, 0xac, 0x02, 1, 2, 3, 4, 5, 6, 7, 8, 11, 12, 13, 14, 15, 16, 17, 18, 2, 9, 10]
        );
        assert_eq!(postcard::from_bytes::<Envelope>(&bytes).unwrap(), e);
        let worst = Envelope {
            timestamp: u64::MAX,
            active: u32::MAX,
            ui: UiHashes { web: [0xff; 8], watch: [0xff; 8] },
            state: &[0xff; STATE_MAX],
        };
        assert_eq!(postcard::to_stdvec(&worst).unwrap().len(), ENVELOPE_MAX);
    }

    #[test]
    fn host_events_on_the_wire() {
        let b = |e: &HostEvent| postcard::to_stdvec(e).unwrap();
        assert_eq!(b(&HostEvent::Select(5)), [0, 5]);
        assert_eq!(b(&HostEvent::SelectDefault), [1]);
        assert_eq!(b(&HostEvent::Engine { package: 5, bytes: &[3] }), [2, 5, 1, 3]);
        assert_eq!(b(&HostEvent::Ble(BleCommand::ForgetAll)), [3, 2]);
        let worst = HostEvent::Engine {
            package: u32::MAX,
            bytes: &[0; EVENT_MAX],
        };
        assert_eq!(b(&worst).len(), HOST_EVENT_MAX);
        // borrowed on the way in
        let bytes = b(&HostEvent::Engine { package: 5, bytes: &[3, 4] });
        match postcard::from_bytes::<HostEvent>(&bytes).unwrap() {
            HostEvent::Engine { package: 5, bytes } => assert_eq!(bytes, &[3, 4]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn choose_view_fits() {
        let mut packages = heapless::Vec::new();
        for i in 0..REGISTRY_MAX {
            packages
                .push(PackageInfo {
                    id: u32::MAX - i as u32,
                    name: Name::try_from("sixteen-chars-xx").unwrap(),
                    version: u16::MAX,
                })
                .unwrap();
        }
        let ble = BleStatusView {
            addr: [0xff; 6],
            pair_ms: u32::MAX,
            bonds: heapless::Vec::from_slice(&[[0xff; 6]; MAX_BLE_BONDS]).unwrap(),
            max_bonds: MAX_BLE_BONDS as u8,
            note: BleNote::PairedNotSaved([0xff; 6]),
        };
        assert_eq!(postcard::to_stdvec(&Some(&ble)).unwrap().len(), BLE_STATUS_VIEW_MAX);
        let view = ChooseView { packages, ble: Some(ble) };
        let n = postcard::to_stdvec(&view).unwrap().len();
        assert!(n <= STATE_MAX, "{n}");
    }

    #[test]
    fn manifest_on_the_wire() {
        let m = Manifest {
            id: package_id("race"),
            name: Name::try_from("race").unwrap(),
            version: 1,
            abi: 1,
            content_hash: [0xaa; 8],
            ui: UiHashes { web: [0xbb; 8], watch: [0xcc; 8] },
            watch_len: 300,
        };
        let bytes: Vec<u8> = postcard::to_stdvec(&m).unwrap();
        assert_eq!(postcard::from_bytes::<Manifest>(&bytes).unwrap(), m);
        let tail = [[0xaa; 8], [0xbb; 8], [0xcc; 8]].concat();
        assert_eq!(&bytes[bytes.len() - 26..bytes.len() - 2], &tail[..]);
        assert_eq!(&bytes[bytes.len() - 2..], &[0xac, 0x02]);
    }

    #[test]
    fn asset_messages_on_the_wire() {
        let get = GetAsset { package: 300, hash: [1, 2, 3, 4, 5, 6, 7, 8] };
        let bytes = postcard::to_stdvec(&get).unwrap();
        assert_eq!(bytes, [0xac, 0x02, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(postcard::from_bytes::<GetAsset>(&bytes).unwrap(), get);
        let worst = GetAsset { package: u32::MAX, hash: [0xff; 8] };
        assert_eq!(postcard::to_stdvec(&worst).unwrap().len(), GET_ASSET_MAX);

        let header = AssetHeader { status: ASSET_OK, len: 20_000 };
        let bytes = postcard::to_stdvec(&header).unwrap();
        assert_eq!(bytes, [0, 0xa0, 0x9c, 0x01]);
        assert_eq!(postcard::from_bytes::<AssetHeader>(&bytes).unwrap(), header);
        let refused = AssetHeader { status: ASSET_HASH_NOT_CURRENT, len: 0 };
        assert_eq!(postcard::to_stdvec(&refused).unwrap(), [2, 0]);
        let worst = AssetHeader { status: 0xff, len: u32::MAX };
        assert_eq!(postcard::to_stdvec(&worst).unwrap().len(), ASSET_HEADER_MAX);
    }
}
