//! Writes the C6 firmware's app image: the file a firmware update uploads
//! (BLE.md "Bundles", `BeginFirmware`), byte for byte what `probe-rs run
//! --idf-partition-table partitions.csv --idf-target-app-partition ota_0`
//! writes into `ota_0`, so its appended digest is the one the box logs at
//! boot (`firmware: running ota_0, ...`) and answers `Firmware` with.
//!
//!     extreme-image <elf> [-o <out>]   # default: target/xiaoc6.bin
//!     extreme-image otadata-blank      # target/otadata-blank.bin
//!
//! The image comes from espflash's library, the release probe-rs builds it
//! with, given what probe-rs gives it for this board: 4 MB of flash (which
//! probe-rs reads from the chip), the bootloader's mode and the C6's
//! default 40 MHz, `tgt-xiaoc6/partitions.csv`, `ota_0`. The image is then
//! checked with `extreme_protocol::firmware::check` and its digest against
//! a SHA-256 of the bytes before it.
//!
//! `otadata-blank` is 8 KiB of 0xFF: written over `otadata` (0xF000) it
//! selects no slot, so the bootloader boots `ota_0` again (the slot
//! probe-rs flashes) after an update has moved the box to `ota_1`.

use std::{fs, path::PathBuf, process::ExitCode};

use espflash::{
    flasher::{FlashData, FlashFrequency, FlashSettings, FlashSize},
    image_format::idf::IdfBootloaderFormat,
    target::{Chip, XtalFrequency},
};
use extreme_protocol::firmware::{self, DIGEST_LEN};
use sha2::{Digest, Sha256};

/// The app slot probe-rs flashes in development.
const APP_PARTITION: &str = "ota_0";
/// `otadata`'s length (partitions.csv): two 4 KiB sectors.
const OTADATA_LEN: usize = 0x2000;
/// esp_app_desc_t's magic word, and where its version string is.
const APP_DESC_MAGIC: u32 = 0xABCD_5432;
const APP_DESC_VERSION: std::ops::Range<usize> = 16..48;

/// The repository's root.
fn root() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::canonicalize(&root).unwrap_or(root)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The app image of the ELF `elf`, as probe-rs writes it into `ota_0`.
fn image(elf: &[u8]) -> Result<Vec<u8>, String> {
    let settings = FlashSettings::new(None, Some(FlashSize::_4Mb), Some(FlashFrequency::_40Mhz));
    let flash = FlashData::new(settings, 0, None, Chip::Esp32c6, XtalFrequency::_40Mhz);
    let table = root().join("tgt-xiaoc6/partitions.csv");
    let format = IdfBootloaderFormat::new(elf, &flash, Some(&table), None, None, Some(APP_PARTITION))
        .map_err(|e| format!("espflash: {e}"))?;
    Ok(format.ota_segments().flat_map(|s| s.data.into_owned()).collect())
}

/// The version in the image's app descriptor (the first segment's start);
/// `None` if it isn't there.
fn version(image: &[u8]) -> Option<String> {
    let desc = image.get(32..32 + APP_DESC_VERSION.end)?;
    if u32::from_le_bytes(desc[..4].try_into().ok()?) != APP_DESC_MAGIC {
        return None;
    }
    let v = &desc[APP_DESC_VERSION];
    let end = v.iter().position(|&b| b == 0).unwrap_or(v.len());
    Some(String::from_utf8_lossy(&v[..end]).into_owned())
}

/// Checks `image` as the box will: its structure, then its digest.
fn self_check(image: &[u8]) -> Result<[u8; 32], String> {
    let found = firmware::check(image.len() as u32, |at, buf: &mut [u8]| {
        buf.copy_from_slice(&image[at as usize..at as usize + buf.len()]);
        Ok::<(), ()>(())
    })
    .map_err(|(reason, detail)| format!("not a valid image: reason {reason}, detail {detail:#x}"))?;
    let at = found.digest_at as usize;
    let digest: [u8; 32] = image[at..at + DIGEST_LEN as usize].try_into().unwrap();
    if Sha256::digest(&image[..at])[..] != digest {
        return Err("the appended digest isn't the SHA-256 of the image".into());
    }
    Ok(digest)
}

fn run(args: &[String]) -> Result<(), String> {
    let usage = "usage: extreme-image <elf> [-o <out>] | extreme-image otadata-blank";
    match args {
        [cmd] if cmd == "otadata-blank" => {
            let out = root().join("target/otadata-blank.bin");
            fs::write(&out, vec![0xFF; OTADATA_LEN]).map_err(|e| format!("{}: {e}", out.display()))?;
            println!("wrote {} ({OTADATA_LEN} B of 0xFF)", out.display());
            Ok(())
        }
        [elf] | [elf, _, _] => {
            let out = match args {
                [_, flag, out] if flag == "-o" => PathBuf::from(out),
                [_] => root().join("target/xiaoc6.bin"),
                _ => return Err(usage.into()),
            };
            let bytes = fs::read(elf).map_err(|e| format!("{elf}: {e}"))?;
            let image = image(&bytes)?;
            let digest = self_check(&image)?;
            fs::write(&out, &image).map_err(|e| format!("{}: {e}", out.display()))?;
            println!("wrote {}", out.display());
            println!("length  {} B", image.len());
            println!("sha256  {}", hex(&digest));
            println!("version {}", version(&image).unwrap_or_else(|| "(no app descriptor)".into()));
            Ok(())
        }
        _ => Err(usage.into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("extreme-image: {e}");
            ExitCode::FAILURE
        }
    }
}
