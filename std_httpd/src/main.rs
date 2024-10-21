use axum::http::header;
use axum::response::IntoResponse;
use axum::{
    body::Bytes,
    extract::Json,
    extract::Path,
    extract::Query,
    routing::{get, post},
    Router,
};
use engine::{EventEngineTrait, Flat, FlatDiff, EventEngine, UpdateResp};
use engine_race::Race;
use hyper::Server;
use hyper::{Body, Response, StatusCode};
use serde::Deserialize;
use serde_json;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{watch, Mutex as TokioMutex};

use tokio::time::{timeout, Duration};

const TIMEOUT: Duration = Duration::from_secs(5);
const TIMEZONE_OFFSET: i64 = (10 * 60 + 30) * 60; // ACDT (ms)
const TIMESTAMP_TOLERANCE_MS: i64 = 20;

type RaceEngine = EventEngine<Race, 1>;

#[derive(Deserialize)]
struct UpdatesQuery {
    cnt: usize,
    timestamp: u64,
}

#[derive(Deserialize)]
struct Location {
    lat: f64,
    lon: f64,
    speed: f64,
    heading: f64,
}

struct EngineState(usize, RaceEngine);
struct SystemState {
    engine: TokioMutex<EngineState>,
    sendr: watch::Sender<String>,
}

#[tokio::main]
async fn main() {
    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));

    let (sender, _) = watch::channel(String::new());
    let system_state = Arc::new(SystemState {
        engine: TokioMutex::new(EngineState(1, RaceEngine::default())),
        sendr: sender,
    });

    let app = Router::new()
        .route( // this is for the test system to feed location data
            "/location",
            post({
                let system_state = system_state.clone();
                move |req: Json<Location>| {
                    let system_state = system_state.clone();
                    async move { location_handler(system_state, req).await }
                }
            }),
        )
        .route(
            "/events",
            post({
                let system_state = system_state.clone();
                move |req: Json<<RaceEngine as EventEngineTrait>::Event>| {
                    let system_state = system_state.clone();
                    async move { events_handler(system_state, req).await }
                }
            }),
        )
        .route(
            "/updates",
            get({
                let system_state = system_state.clone();
                move |query: Query<UpdatesQuery>| {
                    let system_state = system_state.clone();
                    async move { updates_handler(system_state, query).await }
                }
            }),
        )
        .route("/:file", get(static_files_handler));

    let server = Server::bind(&addr).serve(app.into_make_service());
    println!("Server running on http://{}", addr);
    if let Err(e) = server.await {
        eprintln!("Server error: {}", e);
    }
}

use engine_race::RaceStaticFiles;
use engine_race::StaticHttpTrait;

async fn static_files_handler(Path(file): Path<String>) -> Result<impl IntoResponse, StatusCode> {
    println!("file: {}", file);
    match RaceStaticFiles::lookup(&file) {
        Some(data) => {
            let bytes = Bytes::copy_from_slice(data);

            let response = Response::builder()
                .status(StatusCode::OK)
                .body(Body::from(bytes))
                .unwrap();
            Ok(response)
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn location_handler(
    system_state: Arc<SystemState>,
    Json(location): Json<Location>,
) -> Result<&'static str, String> {
    
    let update = {
        let mut engine_state = system_state.engine.lock().await;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_millis() as u64;
        let old_state = engine_state.1.get_state();
        let updated = engine_state.1.update_location(timestamp, Some((location.lat, location.lon)), Some((location.speed, location.heading)));

        if updated {
            engine_state.0 += 1;
            let new_state = engine_state.1.get_state();
            let delta = UpdateResp::new(engine_state.0, FlatDiff(&new_state, &old_state));
            match serde_json::to_string(&delta) {
                Ok(update) => Ok(Some(update)),
                Err(e) => Err(format!("Failed to serialize delta: {}", e)),
            }
        } else {
            Ok(None)
        }
    }?;

    if let Some(update) = update {
        let _ = system_state.sendr.send(update);
    }    
    Ok("OK")    
}

// Define the handler for the `/events` endpoint
async fn events_handler(
    system_state: Arc<SystemState>,
    Json(event): Json<<RaceEngine as EventEngineTrait>::Event>,
) -> Result<&'static str, String> {

    let mut sleep_fn = |_time: u64, _: usize| -> Result<(), &'static str> {
        // Implement your sleep functionality here, if needed
        Ok(())
    };

    let update = {
        let mut engine_state = system_state.engine.lock().await;
        let old_state = engine_state.1.get_state();
        match engine_state.1.handle_event(event, &mut sleep_fn) {
            Ok(updated) => {
                if updated {
                    engine_state.0 += 1;
                    let new_state = engine_state.1.get_state();
                    let delta = UpdateResp::new(engine_state.0, FlatDiff(&new_state, &old_state));
                    match serde_json::to_string(&delta) {
                        Ok(update) => Ok(Some(update)),
                        Err(e) => Err(format!("Failed to serialize delta: {}", e)),
                    }
                } else {
                    Ok(None)
                }
            }
            Err(e) => Err(e.to_string()),
        }
    }?;
    if let Some(update) = update {
        let _ = system_state.sendr.send(update);
    }
    Ok("Done")
}

async fn updates_handler(
    system_state: Arc<SystemState>,
    Query(query): Query<UpdatesQuery>,
) -> Result<impl IntoResponse, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis() as i64;

    let time_offset = now - query.timestamp as i64;
    if query.timestamp != 0 && time_offset.abs() > TIMESTAMP_TOLERANCE_MS {
        let response_data = json!({
            "offset": time_offset,
            "tzOffset": TIMEZONE_OFFSET,
            "cnt": -1,
        });
        let response = Response::builder()
            .status(200)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(response_data.to_string()))
            .unwrap();
        return Ok(response);
    }

    let response = {
        let engine_state = system_state.engine.lock().await;

        if query.cnt < engine_state.0 {
            let state = engine_state.1.get_state();
            let new_state = UpdateResp::new(engine_state.0, Flat(&state));
            let payload = serde_json::to_string(&new_state)
                .map_err(|_| "Failed to serialize state".to_string())?;
            Some(payload)
        } else {
            None
        }
    };

    if let Some(payload) = response {
        let response = Response::builder()
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(payload))
            .unwrap();
        Ok(response)
    } else {
        let mut recvr = system_state.sendr.subscribe();

        match timeout(TIMEOUT, recvr.changed()).await {
            Ok(Ok(())) => {
                let result = recvr.borrow().clone();
                let response = Response::builder()
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(result))
                    .unwrap();
                Ok(response)
            }
            Ok(Err(_)) => Err("Channel closed or all messages dropped".to_string()),
            Err(_) => {
                let response = Response::builder()
                    .status(StatusCode::NO_CONTENT)
                    .body(Body::empty())
                    .unwrap();
                // let now = SystemTime::now()
                //     .duration_since(UNIX_EPOCH)
                //     .expect("Time went backwards")
                //     .as_millis() as i64;

                Ok(response)
            }
        }
    }
}
