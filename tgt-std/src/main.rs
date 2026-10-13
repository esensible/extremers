//! Host target: the package host with Choose and the bundles given on the
//! command line, serving the UIs on port 8080, optionally replaying NMEA.
//!
//!     cargo run -p tgt-std -- --bundle target/packages/race.bundle [--bundle …] [--gps FILE]
//!
//! `--bundle PATH` (repeatable, or several paths after one flag) loads a
//! bundle file (`cargo run -p extreme-pack -- --bundle` writes them);
//! `--gps FILE` feeds the file's NMEA to the host, one line a second, as a
//! receiver at 1 Hz would (a `$GxRMC` per line; other lines pass through the
//! parser and are dropped). Without it there is no GPS.

use std::{path::PathBuf, process::ExitCode};

use edge_nal_std::Stack;
use embassy_executor::Executor;
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;

use common::{
    engine::{EngineMemory, LIMITS},
    host::{Host, Registry},
    nmea::AsyncReader,
    package::Package,
    tasks::{read_gps, serve_http},
};

/// Port 80 needs privileges on a host, so use the conventional alternative.
const HTTP_PORT: u16 = 8080;

/// The bundle at `path`, loaded (and kept for the life of the program).
fn load(path: &PathBuf, memory: &mut EngineMemory) -> Result<Package, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    Package::load(bytes, &LIMITS, &mut memory.scratch).map_err(|e| format!("{}: not a valid bundle: {e:?}", path.display()))
}

fn main() -> ExitCode {
    // RUST_LOG=debug for the host's timer, GPS and evaluation tracing
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // --bundle A [B …] [--bundle C] [--gps FILE]
    let mut bundles = Vec::new();
    let mut gps = None;
    let mut flag = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--bundle" | "--gps" => flag = Some(arg),
            _ => match flag.as_deref() {
                Some("--bundle") => bundles.push(PathBuf::from(arg)),
                Some("--gps") if gps.is_none() => gps = Some(PathBuf::from(arg)),
                _ => {
                    eprintln!("usage: tgt-std [--bundle PATH…] [--gps FILE]");
                    return ExitCode::FAILURE;
                }
            },
        }
    }

    let memory: &'static mut EngineMemory = Box::leak(Box::new(EngineMemory::new()));
    let choose = match Package::load(extreme_packages::CHOOSE, &LIMITS, &mut memory.scratch) {
        Ok(choose) => choose,
        Err(e) => {
            eprintln!("Choose's bundle is not valid: {e:?}");
            return ExitCode::FAILURE;
        }
    };
    let mut apps = Vec::new();
    for path in &bundles {
        match load(path, memory) {
            Ok(package) => {
                log::info!("{}: {} v{} (id {})", path.display(), package.name, package.version, package.id);
                apps.push(package);
            }
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
    }
    let nmea = match gps.map(|path| std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))) {
        Some(Ok(bytes)) => Some(bytes),
        Some(Err(e)) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
        None => None,
    };

    static HOST: StaticCell<Host> = StaticCell::new();
    let host: &'static Host = HOST.init(Host::new(Registry::new(Some(choose), &apps), memory));

    static STACK: StaticCell<Stack> = StaticCell::new();
    let stack: &'static Stack = STACK.init(Stack::new());

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        match httpd_task(stack, host) {
            Ok(token) => spawner.spawn(token),
            Err(_) => log::warn!("failed to spawn httpd task"),
        }
        match timer_task(host) {
            Ok(token) => spawner.spawn(token),
            Err(_) => log::warn!("failed to spawn timer task"),
        }
        if let Some(nmea) = nmea {
            match gps_task(Replay { lines: nmea, at: 0 }, host) {
                Ok(token) => spawner.spawn(token),
                Err(_) => log::warn!("failed to spawn GPS task"),
            }
        }
    })
}

#[embassy_executor::task]
async fn httpd_task(stack: &'static Stack, host: &'static Host) -> ! {
    serve_http(stack, HTTP_PORT, host).await
}

#[embassy_executor::task]
async fn timer_task(host: &'static Host) -> ! {
    host.run_timer().await
}

#[embassy_executor::task]
async fn gps_task(replay: Replay, host: &'static Host) -> ! {
    read_gps(replay, host).await
}

/// An NMEA file as a receiver at 1 Hz: a line a second. At the end it
/// stays silent (no more fixes), like a receiver that lost its fix.
struct Replay {
    lines: Vec<u8>,
    /// Where the next line starts.
    at: usize,
}

impl AsyncReader for Replay {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        loop {
            if self.at < self.lines.len() {
                break;
            }
            if self.at == self.lines.len() {
                log::info!("gps: replay done");
                self.at += 1;
            }
            Timer::after(Duration::from_secs(3600)).await;
        }
        // a new line a second after the last; then the line, as much as fits
        if self.at > 0 && self.lines[self.at - 1] == b'\n' {
            Timer::after(Duration::from_secs(1)).await;
        }
        let line_end = self.lines[self.at..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(self.lines.len(), |i| self.at + i + 1);
        let n = (line_end - self.at).min(buf.len());
        buf[..n].copy_from_slice(&self.lines[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}
