//! BLE identity, bonds and pairing control, as far as they are plain data
//! (BLE.md "Identity", "Security", "Pairing a watch").
//!
//! Shared by the BLE transport ([`crate::ble`], C6 only), the web transport
//! ([`crate::http`]: the Kindle's settings panel opens the pairing window and
//! shows the bonds) and the target's flash storage (bond records). Nothing
//! here needs trouble-host, so it builds and is tested on every target.

use core::fmt;

use embassy_time::Instant;
use postcard_schema::Schema;
use serde::{Deserialize, Serialize};

use crate::config::MAX_BLE_BONDS;

/// The device's BLE static random address from the chip's MAC address.
///
/// `mac` is in the order it is printed (most significant byte first); the
/// result is in BLE (and trouble-host `BdAddr`) byte order, least
/// significant first. A static random address has the two top bits of its
/// most significant byte set (Core Vol 6 Part B 1.3.2.1), so those two bits
/// of the MAC are lost; the other 46 keep every MAC distinct. The spec also
/// forbids the 46 random bits being all 0 or all 1, which no real MAC is,
/// but the lowest bit is flipped if one ever were.
pub fn static_random_address(mac: [u8; 6]) -> [u8; 6] {
    let mut addr = mac;
    addr.reverse();
    addr[5] |= 0xc0;
    let degenerate = addr[..5].iter().all(|&b| b == 0) && addr[5] == 0xc0
        || addr[..5].iter().all(|&b| b == 0xff) && addr[5] == 0xff;
    if degenerate {
        addr[0] ^= 1;
    }
    addr
}

/// A BLE address (BLE byte order) shown as `AA:BB:CC:DD:EE:FF`, most
/// significant byte first, the way devices and tools print it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AddrText(pub [u8; 6]);

impl fmt::Display for AddrText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let a = self.0;
        write!(
            f,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            a[5], a[4], a[3], a[2], a[1], a[0]
        )
    }
}

impl fmt::Debug for AddrText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// One bonded device as stored in flash: what trouble-host needs to
/// recognise it (identity address, IRK) and to resume encryption (LTK).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BondRecord {
    /// Identity address kind as on air: 0 public, 1 random.
    pub addr_kind: u8,
    /// Identity address, BLE byte order.
    pub addr: [u8; 6],
    /// The peer's identity resolving key, if it sent one (Apple devices do;
    /// it is how a peer using resolvable private addresses is recognised).
    pub irk: Option<[u8; 16]>,
    /// Long term key, little-endian as trouble-host keeps it.
    pub ltk: [u8; 16],
    /// Paired with MITM protection. Always false here (Just Works).
    pub authenticated: bool,
    /// When it bonded, as a counter: the lowest is replaced first when all
    /// [`MAX_BLE_BONDS`](crate::config::MAX_BLE_BONDS) slots are taken.
    pub seq: u32,
}

/// Encoded size of a [`BondRecord`].
pub const BOND_RECORD_LEN: usize = 46;
/// Version byte of the record layout below.
const BOND_RECORD_VERSION: u8 = 1;

impl BondRecord {
    /// `[1][addr_kind][addr 6][has_irk][irk 16][ltk 16][authenticated][seq u32 LE]`.
    pub fn encode(&self) -> [u8; BOND_RECORD_LEN] {
        let mut out = [0u8; BOND_RECORD_LEN];
        out[0] = BOND_RECORD_VERSION;
        out[1] = self.addr_kind;
        out[2..8].copy_from_slice(&self.addr);
        if let Some(irk) = self.irk {
            out[8] = 1;
            out[9..25].copy_from_slice(&irk);
        }
        out[25..41].copy_from_slice(&self.ltk);
        out[41] = self.authenticated as u8;
        out[42..46].copy_from_slice(&self.seq.to_le_bytes());
        out
    }

    /// `None` for anything that is not exactly a version-1 record.
    pub fn decode(b: &[u8]) -> Option<Self> {
        let b: &[u8; BOND_RECORD_LEN] = b.try_into().ok()?;
        if b[0] != BOND_RECORD_VERSION || b[1] > 1 || b[8] > 1 || b[41] > 1 {
            return None;
        }
        let mut addr = [0u8; 6];
        addr.copy_from_slice(&b[2..8]);
        let irk = (b[8] == 1).then(|| {
            let mut irk = [0u8; 16];
            irk.copy_from_slice(&b[9..25]);
            irk
        });
        let mut ltk = [0u8; 16];
        ltk.copy_from_slice(&b[25..41]);
        Some(Self {
            addr_kind: b[1],
            addr,
            irk,
            ltk,
            authenticated: b[41] == 1,
            seq: u32::from_le_bytes([b[42], b[43], b[44], b[45]]),
        })
    }
}

/// What the Kindle's settings panel asks of the BLE transport: the `Ble`
/// variant of a websocket event ([`WsEvent`](crate::protocol::WsEvent)).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Schema)]
pub enum BleCommand {
    /// Open the pairing window for
    /// [`BLE_PAIRING_WINDOW_SECS`](crate::config::BLE_PAIRING_WINDOW_SECS).
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

/// The BLE side's state as the settings panel shows it.
#[derive(Clone, Debug, Default)]
pub struct BleStatus {
    /// This box's BLE address (BLE byte order).
    pub addr: [u8; 6],
    /// When the pairing window closes; `None` while closed.
    pub pair_until: Option<Instant>,
    /// Identity addresses of the bonded devices, oldest first.
    pub bonds: heapless::Vec<[u8; 6], MAX_BLE_BONDS>,
    pub note: BleNote,
}

/// [`BleStatus`] as the websocket clients get it: the `ble` field of a
/// [`WsState`](crate::protocol::WsState) (BLE.md "Pairing a watch").
/// Addresses are in BLE byte order (least significant byte first): print
/// them reversed, as [`AddrText`] does.
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
/// `status_view_worst_case_fits` checks it.
pub const BLE_STATUS_VIEW_MAX: usize = 1 + 6 + 5 + 1 + 6 * MAX_BLE_BONDS + 1 + 1 + 6;

impl BleStatus {
    /// The view of this status at `now` (the pairing window's time left is
    /// relative to it).
    pub fn view(&self, now: Instant) -> BleStatusView {
        let pair_ms = self
            .pair_until
            .map(|until| until.saturating_duration_since(now).as_millis())
            .unwrap_or(0);
        BleStatusView {
            addr: self.addr,
            pair_ms: pair_ms.min(u32::MAX as u64) as u32,
            bonds: self.bonds.clone(),
            max_bonds: MAX_BLE_BONDS as u8,
            note: self.note,
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use embassy_time::Duration;
    use std::string::ToString;

    #[test]
    fn address_from_mac() {
        let addr = static_random_address([0x54, 0x32, 0x04, 0x01, 0x02, 0x03]);
        assert_eq!(addr, [0x03, 0x02, 0x01, 0x04, 0x32, 0xd4]);
        assert_eq!(addr[5] & 0xc0, 0xc0);
        assert_eq!(AddrText(addr).to_string(), "D4:32:04:01:02:03");
        // two units differ in their last bytes, and so do their addresses
        assert_ne!(addr, static_random_address([0x54, 0x32, 0x04, 0x01, 0x02, 0x04]));
        // the forbidden all-ones random part
        let ones = static_random_address([0xff; 6]);
        assert_eq!(ones[5] & 0xc0, 0xc0);
        assert_ne!(ones, [0xff; 6]);
    }

    #[test]
    fn bond_record_round_trip() {
        let mut record = BondRecord {
            addr_kind: 1,
            addr: [1, 2, 3, 4, 5, 0xc6],
            irk: Some([7; 16]),
            ltk: [9; 16],
            authenticated: false,
            seq: 0x01020304,
        };
        let bytes = record.encode();
        assert_eq!(BondRecord::decode(&bytes), Some(record));
        record.irk = None;
        assert_eq!(BondRecord::decode(&record.encode()), Some(record));
        // wrong length or version: nothing
        assert_eq!(BondRecord::decode(&bytes[..45]), None);
        let mut bad = bytes;
        bad[0] = 2;
        assert_eq!(BondRecord::decode(&bad), None);
    }

    #[test]
    fn commands_on_the_wire() {
        assert_eq!(postcard::to_stdvec(&BleCommand::Pair).unwrap(), [0]);
        assert_eq!(postcard::to_stdvec(&BleCommand::Cancel).unwrap(), [1]);
        assert_eq!(postcard::to_stdvec(&BleCommand::ForgetAll).unwrap(), [2]);
        assert_eq!(postcard::from_bytes::<BleCommand>(&[2]), Ok(BleCommand::ForgetAll));
        assert!(postcard::from_bytes::<BleCommand>(&[3]).is_err());
    }

    #[test]
    fn status_view() {
        let now = Instant::from_secs(1000);
        let mut status = BleStatus {
            addr: [3, 2, 1, 4, 0x32, 0xd4],
            ..Default::default()
        };
        let view = status.view(now);
        assert_eq!(view.pair_ms, 0);
        assert_eq!(view.max_bonds, MAX_BLE_BONDS as u8);
        assert_eq!(
            postcard::to_stdvec(&view).unwrap(),
            [3, 2, 1, 4, 0x32, 0xd4, 0, 0, MAX_BLE_BONDS as u8, 0]
        );
        status.pair_until = Some(now + Duration::from_millis(117_500));
        status.bonds.push([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]).unwrap();
        status.bonds.push([1, 2, 3, 4, 5, 6]).unwrap();
        status.note = BleNote::Paired([1, 2, 3, 4, 5, 6]);
        let view = status.view(now);
        assert_eq!(view.pair_ms, 117_500);
        assert_eq!(&view.bonds[..], &[[0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff], [1, 2, 3, 4, 5, 6]]);
        assert_eq!(view.note, BleNote::Paired([1, 2, 3, 4, 5, 6]));
        let bytes = postcard::to_stdvec(&view).unwrap();
        assert_eq!(postcard::from_bytes::<BleStatusView>(&bytes).unwrap(), view);
        // a window that has run out reads as closed
        assert_eq!(status.view(now + Duration::from_secs(200)).pair_ms, 0);
    }

    #[test]
    fn status_view_worst_case_fits() {
        let mut status = BleStatus {
            addr: [0xff; 6],
            pair_until: Some(Instant::MAX),
            note: BleNote::PairedNotSaved([0xff; 6]),
            ..Default::default()
        };
        for _ in 0..MAX_BLE_BONDS {
            status.bonds.push([0xff; 6]).unwrap();
        }
        let view = Some(status.view(Instant::from_secs(0)));
        assert_eq!(postcard::to_stdvec(&view).unwrap().len(), BLE_STATUS_VIEW_MAX);
    }
}
