//! The BLE bulk channel's messages and its streaming, as plain data (BLE.md
//! "Watch bundles"): what a request may be, and how an answer is cut into
//! SDUs. The transport (`ble.rs`, an L2CAP connection-oriented channel)
//! moves the bytes; nothing here needs trouble-host, so it is tested on
//! every target.
//!
//! A client sends one [`GetAsset`] per SDU and gets back an
//! [`AssetHeader`] (its own SDU) and then exactly `len` bytes of the asset
//! in SDUs of at most the size the link allows. The asset is `'static` (the
//! package in flash): every data SDU is a slice of it, never a copy.

use extreme_protocol::{ASSET_HEADER_MAX, AssetHeader, GET_ASSET_MAX, GetAsset};

/// Why a request was refused: the channel is closed after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadRequest {
    /// Not exactly one `GetAsset` (short, trailing bytes, an id past u32).
    Undecodable,
    /// Longer than any `GetAsset` can be.
    TooLong,
}

/// A client's request: exactly one postcard [`GetAsset`].
pub fn parse_request(bytes: &[u8]) -> Result<GetAsset, BadRequest> {
    if bytes.len() > GET_ASSET_MAX {
        return Err(BadRequest::TooLong);
    }
    match postcard::take_from_bytes::<GetAsset>(bytes) {
        Ok((request, [])) => Ok(request),
        _ => Err(BadRequest::Undecodable),
    }
}

/// One SDU of an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sdu {
    /// The [`AssetHeader`], postcard.
    Header(heapless::Vec<u8, ASSET_HEADER_MAX>),
    /// The next bytes of the asset, straight from where it is stored.
    Data(&'static [u8]),
}

impl Sdu {
    pub fn bytes(&self) -> &[u8] {
        match self {
            Sdu::Header(h) => h,
            Sdu::Data(d) => d,
        }
    }
}

/// An answer being sent: the header, then the asset in SDUs of at most
/// `sdu_max` bytes. An iterator of [`Sdu`]s; [`sent`](Self::sent) says how
/// far it got.
#[derive(Clone, Debug)]
pub struct Transfer {
    header: AssetHeader,
    asset: &'static [u8],
    sdu_max: usize,
    header_sent: bool,
    sent: usize,
}

impl Transfer {
    /// The answer `header` with `asset` (empty unless the header says ok;
    /// its length is the header's `len`), in SDUs of `sdu_max` bytes (at
    /// least 1).
    pub fn new(header: AssetHeader, asset: &'static [u8], sdu_max: usize) -> Self {
        debug_assert_eq!(header.len as usize, asset.len());
        Self {
            header,
            asset,
            sdu_max: sdu_max.max(1),
            header_sent: false,
            sent: 0,
        }
    }

    pub fn header(&self) -> AssetHeader {
        self.header
    }

    /// Asset bytes handed out so far.
    pub fn sent(&self) -> usize {
        self.sent
    }

    /// Whether every SDU has been handed out.
    pub fn done(&self) -> bool {
        self.header_sent && self.sent == self.asset.len()
    }

    /// How many SDUs the whole answer takes, the header's included.
    pub fn sdus(&self) -> usize {
        1 + self.asset.len().div_ceil(self.sdu_max)
    }
}

impl Iterator for Transfer {
    type Item = Sdu;

    fn next(&mut self) -> Option<Sdu> {
        if !self.header_sent {
            self.header_sent = true;
            let mut header = heapless::Vec::new();
            // cannot fail: the buffer holds the largest header
            let _ = header.resize(ASSET_HEADER_MAX, 0);
            let n = postcard::to_slice(&self.header, &mut header).map_or(0, |b| b.len());
            header.truncate(n);
            return Some(Sdu::Header(header));
        }
        if self.sent == self.asset.len() {
            return None;
        }
        let end = (self.sent + self.sdu_max).min(self.asset.len());
        let chunk = &self.asset[self.sent..end];
        self.sent = end;
        Some(Sdu::Data(chunk))
    }
}

/// The largest data SDU for a channel whose peer takes SDUs of `peer_mtu`
/// and PDUs of `peer_mps` bytes, sent from packets of `packet` bytes (the
/// host's packet pool): one SDU per PDU, so each costs one credit and one
/// packet. The PDU carries a 4-byte L2CAP header and the SDU's 2-byte
/// length before the data.
pub fn sdu_max(peer_mtu: u16, peer_mps: u16, packet: usize) -> usize {
    (peer_mtu as usize)
        .min((peer_mps as usize).saturating_sub(2))
        .min(packet.saturating_sub(6))
}

/// Bytes per second for `bytes` in `ms` milliseconds (0 ms counts as 1).
pub fn throughput(bytes: usize, ms: u64) -> u64 {
    bytes as u64 * 1000 / ms.max(1)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use extreme_protocol::{ASSET_HASH_NOT_CURRENT, ASSET_OK};
    use std::vec::Vec;

    static ASSET: [u8; 1000] = {
        let mut a = [0u8; 1000];
        let mut i = 0;
        while i < a.len() {
            a[i] = (i % 251) as u8;
            i += 1;
        }
        a
    };

    #[test]
    fn requests() {
        let get = GetAsset { package: 2394669720, hash: [1, 2, 3, 4, 5, 6, 7, 8] };
        let bytes = postcard::to_stdvec(&get).unwrap();
        assert_eq!(parse_request(&bytes), Ok(get));
        // short, trailing byte, an id past u32, nothing, too long
        assert_eq!(parse_request(&bytes[..bytes.len() - 1]), Err(BadRequest::Undecodable));
        // a trailing byte: on a short request, and past the longest one
        let mut trailing = postcard::to_stdvec(&GetAsset { package: 5, hash: [0; 8] }).unwrap();
        trailing.push(0);
        assert_eq!(parse_request(&trailing), Err(BadRequest::Undecodable));
        let mut long = bytes.clone();
        long.push(0);
        assert_eq!(parse_request(&long), Err(BadRequest::TooLong));
        let past_u32 = [0xff, 0xff, 0xff, 0xff, 0x1f, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(parse_request(&past_u32), Err(BadRequest::Undecodable));
        assert_eq!(parse_request(&[]), Err(BadRequest::Undecodable));
        assert_eq!(parse_request(&[0; GET_ASSET_MAX + 1]), Err(BadRequest::TooLong));
    }

    fn all(transfer: Transfer) -> Vec<Sdu> {
        transfer.collect()
    }

    #[test]
    fn header_then_the_asset_in_sdus() {
        let header = AssetHeader { status: ASSET_OK, len: 1000 };
        let t = Transfer::new(header, &ASSET, 245);
        assert_eq!(t.sdus(), 1 + 5);
        let sdus = all(t);
        assert_eq!(sdus.len(), 6);
        assert_eq!(sdus[0], Sdu::Header(heapless::Vec::from_slice(&[0, 0xe8, 0x07]).unwrap()));
        let sizes: Vec<_> = sdus[1..].iter().map(|s| s.bytes().len()).collect();
        assert_eq!(sizes, [245, 245, 245, 245, 20]);
        // the data, in order, is the asset itself (slices of it, no copy)
        let data: Vec<u8> = sdus[1..].iter().flat_map(|s| s.bytes().iter().copied()).collect();
        assert_eq!(data, ASSET);
        let Sdu::Data(first) = sdus[1] else { panic!() };
        assert_eq!(first.as_ptr(), ASSET.as_ptr());
    }

    #[test]
    fn exact_multiple_and_tiny_sdus() {
        let header = AssetHeader { status: ASSET_OK, len: 980 };
        let sizes: Vec<_> = all(Transfer::new(header, &ASSET[..980], 245)).iter().skip(1).map(|s| s.bytes().len()).collect();
        assert_eq!(sizes, [245; 4]);
        // an SDU size of 0 is taken as 1
        let header = AssetHeader { status: ASSET_OK, len: 3 };
        assert_eq!(all(Transfer::new(header, &ASSET[..3], 0)).len(), 4);
    }

    #[test]
    fn a_refusal_or_an_empty_asset_is_the_header_alone() {
        let header = AssetHeader { status: ASSET_HASH_NOT_CURRENT, len: 0 };
        let mut t = Transfer::new(header, &[], 245);
        assert_eq!(t.sdus(), 1);
        assert!(!t.done());
        assert_eq!(t.next(), Some(Sdu::Header(heapless::Vec::from_slice(&[2, 0]).unwrap())));
        assert!(t.done());
        assert_eq!(t.next(), None);
        assert_eq!(t.sent(), 0);
        let empty = AssetHeader { status: ASSET_OK, len: 0 };
        assert_eq!(all(Transfer::new(empty, &[], 245)).len(), 1);
    }

    #[test]
    fn progress() {
        let header = AssetHeader { status: ASSET_OK, len: 500 };
        let mut t = Transfer::new(header, &ASSET[..500], 200);
        t.next();
        assert_eq!(t.sent(), 0);
        t.next();
        assert_eq!(t.sent(), 200);
        t.next();
        t.next();
        assert_eq!(t.sent(), 500);
        assert!(t.done());
        assert_eq!(t.next(), None);
    }

    #[test]
    fn sdu_sizes() {
        // iOS-like peer: large MTU, MPS bigger than a packet: the packet
        // (251 bytes: 4 header + 2 length + 245) is the limit
        assert_eq!(sdu_max(2048, 1004, 251), 245);
        // a peer MPS smaller than a packet: one PDU per SDU
        assert_eq!(sdu_max(2048, 100, 251), 98);
        // a peer MTU smaller still
        assert_eq!(sdu_max(64, 247, 251), 64);
        // degenerate peers never underflow
        assert_eq!(sdu_max(23, 1, 251), 0);
    }

    #[test]
    fn rates() {
        assert_eq!(throughput(20_000, 1000), 20_000);
        assert_eq!(throughput(20_000, 500), 40_000);
        assert_eq!(throughput(10, 0), 10_000);
    }
}
