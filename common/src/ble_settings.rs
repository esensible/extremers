//! BLE identity, as far as it is plain data (BLE.md "Identity").
//!
//! Nothing here needs trouble-host, so it builds and is tested on every
//! target.

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
}
