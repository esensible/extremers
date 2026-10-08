//! BLE identity and bonds, as far as they are plain data (BLE.md
//! "Identity", "Security").
//!
//! Shared by the BLE transport ([`crate::ble`], C6 only) and the target's
//! flash storage (bond records). Nothing here needs trouble-host, so it
//! builds and is tested on every target.

use core::fmt;

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

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
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
}
