use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use actix_web::{App, HttpResponse, HttpServer, post, put, web};
use log::info;

use amqp_service::bin_utils::Properties;
use amqp_service::rest;
use amqp_service::rest::{create_named_worker, HttpSettings};

const CARGO_BIN_NAME: &str = env!("CARGO_BIN_NAME");

struct HttpWorkerSettings {
    counter: Mutex<u64>,
    log_per_request: u64,
    name: String,
}

struct SinkProperties {
    base: Properties,
}

impl SinkProperties {
    fn new() -> Self {
        let base = Properties::new("main", "SINK");
        SinkProperties {
            base
        }
    }

    fn init_version_string(&self, http_settings: &HttpSettings, log_per_request: u64) -> String {
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
        self.base.init_version(CARGO_BIN_NAME, additional_params)
    }

    fn init_http_log_per_request(&self) -> u64 {
        let key = self.base.prop_name("HTTP_LOG_PER_REQUEST");
        let log_per_request = std::env::var(&key)
            .unwrap_or_else(|_| "0".into())
            .parse::<u64>()
            .expect(
                &format!("Cannot parse '{key}' environment variable. It needs to be a non-negative number.")
                );

        info!(target: &self.base.log_target, "Value for '{key}': {log_per_request}");

        log_per_request
    }
}

#[tokio::main]
async fn main() {
    let start = Instant::now();

    let properties = SinkProperties::new();
    properties.base.init_logging();

    info!(target: &properties.base.log_target, "Service '{CARGO_BIN_NAME}' is starting...");

    let http_settings = properties.base.init_http_settings_with_core_count();
    let log_per_request = properties.init_http_log_per_request();
    let version_string = properties.init_version_string(&http_settings, log_per_request);

    info!(target: &properties.base.log_target, "Binding HTTP server to '{}' on {} thread(s).", &http_settings.http_addr, http_settings.http_workers);

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

    info!(target: &properties.base.log_target, "Service inited in {:?}", start.elapsed());

    server
        .run()
        .await
        .expect("HTTP server could not be started");
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
