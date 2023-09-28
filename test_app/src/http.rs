#![feature(associated_type_defaults)]

use std::sync::Arc;
use std::net::SocketAddr;
use tokio::sync::broadcast;
use axum::{Router, handler::{get, post}, response::IntoResponse};
use hyper::{Server, Body, Response};
use serde_json;

mod engine_traits;
mod engine_context;
mod race;

use engine_context::EngineContext;
use race::Race;


#[tokio::main]
async fn main() {
    let (tx, _rx) = broadcast::channel::<String>(100); // The broadcast channel for all requests
    let shared_tx = Arc::new(tx);

    let cloned_tx = shared_tx.clone();
    let engine_context = Arc::new(EngineContext::default());
    engine_context.set_engine(Race::default(), Box::new(move |message| {
        release_states(cloned_tx.clone(), message);
    }));
    
    let events_engine = engine_context.clone();
    let app = Router::new()
        .route("/events", post(move |body: String| handle_events(events_engine, body)))
        .route("/states", get(move || handle_states(shared_tx)));

    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));

    println!("Server listening on http://{}", addr);

    Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .expect("Server failed");
}

async fn handle_events(engine: Arc<EngineContext>, body: String) -> impl axum::response::IntoResponse {
    let _ = engine.handle_event(&body);
    axum::response::Json(serde_json::json!({"status": "success"}))
}

async fn handle_states(shared_tx: Arc<broadcast::Sender<String>>) -> impl IntoResponse {
    let mut rx = shared_tx.subscribe();
    let response = rx.recv().await.expect("Failed to receive response");
    Response::new(Body::from(response))
}

fn release_states(shared_tx: Arc<broadcast::Sender<String>>, message: String) {
    let _ = shared_tx.send(message);  // Sends to all subscribers
} 
