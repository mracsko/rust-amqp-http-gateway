use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use actix_web::{post, put, web, App, HttpResponse, HttpServer};
use log::info;

use amqp_service::bin_utils::{init_http_settings, init_logging, init_version};
use amqp_service::rest;
use amqp_service::rest::{create_named_worker, HttpSettings};

const CARGO_BIN_NAME: &'static str = env!("CARGO_BIN_NAME");

struct HttpWorkerSettings {
    counter: Mutex<u64>,
    log_per_request: u64,
    name: String,
}

#[tokio::main]
async fn main() {
    let start = Instant::now();
    init_logging();
    info!(target: "main", "Service '{CARGO_BIN_NAME}' is starting...");

    let http_settings = init_http_settings(num_cpus::get_physical());
    let log_per_request = init_http_log_per_request();
    let version_string = init_version_string(&http_settings, log_per_request);

    info!(target: "main", "Binding HTTP server to '{}' on {} thread(s).", &http_settings.http_addr, http_settings.http_workers);

    let worker_counter = Arc::new(Mutex::new(0));
    let server = HttpServer::new(move || {
        let name = create_named_worker(&worker_counter, "sink", &start);

        App::new()
            .app_data(web::Data::new(rest::VersionJson {
                json: version_string.clone(),
            }))
            .app_data(web::Data::new(HttpWorkerSettings {
                counter: Mutex::new(0u64),
                log_per_request,
                name,
            }))
            .service(rest::version)
            .service(sink_put)
            .service(sink_post)
    })
    .workers(http_settings.http_workers)
    .bind(http_settings.http_addr)
    .expect("Cannot bind HTTP server");

    info!(target: "main", "Service inited in {:?}", start.elapsed());

    server
        .run()
        .await
        .expect("HTTP server could not be started");
}

fn init_version_string(http_settings: &HttpSettings, log_per_request: u64) -> String {
    let additional_params: HashMap<String, String> = HashMap::from([
        ("cores".to_string(), num_cpus::get_physical().to_string()),
        (
            "http_log_per_request".to_string(),
            log_per_request.to_string(),
        ),
        (
            "http_workers".to_string(),
            http_settings.http_workers.to_string(),
        ),
    ]);
    let version_string = init_version(&CARGO_BIN_NAME, additional_params);
    version_string
}

fn init_http_log_per_request() -> u64 {
    let log_per_request = std::env::var("HTTP_LOG_PER_REQUEST")
        .unwrap_or_else(|_| "0".into())
        .parse::<u64>()
        .expect(
            "Cannot parse 'HTTP_LOG_PER_REQUEST' environment variable. It needs to be a non-negative number.",
        );

    info!(target: "main", "Value for 'HTTP_LOG_PER_REQUEST': {log_per_request}");
    log_per_request
}

#[put("/sink")]
async fn sink_put(settings: web::Data<HttpWorkerSettings>) -> HttpResponse {
    sink(settings)
}

#[post("/sink")]
async fn sink_post(settings: web::Data<HttpWorkerSettings>) -> HttpResponse {
    sink(settings)
}

fn sink(settings: web::Data<HttpWorkerSettings>) -> HttpResponse {
    let mut counter = settings
        .counter
        .lock()
        .expect("Illegal state, cannot acquire lock");
    *counter += 1;

    if settings.log_per_request != 0 && *counter % settings.log_per_request == 0 {
        info!(target: &settings.name, "Sinking messages...");
    }

    HttpResponse::Ok().finish()
}
