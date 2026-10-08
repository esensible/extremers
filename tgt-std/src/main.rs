//! Host target: serves the web UI on port 8080, with no GPS.

use edge_nal_std::Stack;
use embassy_executor::Executor;
use static_cell::StaticCell;

use common::{runtime::EngineRuntime, tasks::serve_http};
// The engines, and their order on the wire: extreme-engines.
use extreme_engines::EngineType;

type Runtime = EngineRuntime<EngineType>;

/// Port 80 needs privileges on a host, so use the conventional alternative.
const HTTP_PORT: u16 = 8080;

fn main() {
    // RUST_LOG=debug for the runtime's timer and GPS tracing
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    static RUNTIME: StaticCell<Runtime> = StaticCell::new();
    let runtime: &'static Runtime = RUNTIME.init(EngineRuntime::new(EngineType::default()));

    static STACK: StaticCell<Stack> = StaticCell::new();
    let stack: &'static Stack = STACK.init(Stack::new());

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        match httpd_task(stack, runtime) {
            Ok(token) => spawner.spawn(token),
            Err(_) => log::warn!("failed to spawn httpd task"),
        }

        match timer_task(runtime) {
            Ok(token) => spawner.spawn(token),
            Err(_) => log::warn!("failed to spawn timer task"),
        }
    });
}

#[embassy_executor::task]
async fn httpd_task(stack: &'static Stack, runtime: &'static Runtime) -> ! {
    serve_http(stack, HTTP_PORT, runtime).await
}

#[embassy_executor::task]
async fn timer_task(runtime: &'static Runtime) -> ! {
    runtime.run_timer().await
}
