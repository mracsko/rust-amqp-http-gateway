use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use actix_web::{App, error, Error, HttpResponse, HttpServer, post, web};
use futures_lite::StreamExt;
use log::{error, info};

use amqp_service::amqp::{AmqpConnectionPool, AmqpChannelWrapper};
use amqp_service::bin_utils::{init_amqp_conn_pool, init_http_settings, init_logging, init_version};
use amqp_service::rest;
use amqp_service::rest::{create_named_worker, HttpSettings};

const CARGO_BIN_NAME: &'static str = env!("CARGO_BIN_NAME");
const MAX_SIZE: usize = 262_144; // max payload size is 256k

struct HttpWorkerSettings {
    name: String,
}

#[tokio::main]
async fn main() {
    let start = Instant::now();
    init_logging();

    info!(target: "main", "Service '{CARGO_BIN_NAME}' is starting...");

    let http_settings = init_http_settings(num_cpus::get_physical());
    let amqp_pool = init_amqp_conn_pool().await;
    let version_string = init_version_string(&http_settings, &amqp_pool);

    info!(target: "main", "Binding HTTP server to '{}' on {} thread(s).", &http_settings.http_addr, http_settings.http_workers);

    let worker_counter = Arc::new(Mutex::new(0));
    let server = HttpServer::new(move || {
        let name = create_named_worker(&worker_counter, "producer", &start);

        App::new()
            .app_data(web::Data::new(rest::VersionJson {
                json: version_string.clone(),
            }))
            .app_data(web::Data::new(Mutex::new(amqp_pool.new_wrapper(&name))))
            .app_data(web::Data::new(HttpWorkerSettings { name }))
            .service(rest::version)
            .service(send)
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

fn init_version_string(http_settings: &HttpSettings, amqp_pool: &AmqpConnectionPool) -> String {
    let additional_params: HashMap<String, String> = HashMap::from([
        ("cores".to_string(), num_cpus::get_physical().to_string()),
        (
            "http_workers".to_string(),
            http_settings.http_workers.to_string(),
        ),
        ("amqp_queue".to_string(), amqp_pool.queue_name.to_string()),
    ]);
    let version_string = init_version(&CARGO_BIN_NAME, additional_params);
    version_string
}

#[post("/send")]
async fn send(
    mut payload: web::Payload,
    channel: web::Data<Mutex<AmqpChannelWrapper>>,
    http_settings: web::Data<HttpWorkerSettings>,
) -> Result<HttpResponse, Error> {
    let mut body = web::BytesMut::new();
    while let Some(chunk) = payload.next().await {
        let chunk = chunk?;
        if (body.len() + chunk.len()) > MAX_SIZE {
            return Err(error::ErrorBadRequest("overflow"));
        }
        body.extend_from_slice(&chunk);
    }

    let mut channel = match channel.lock() {
        Ok(channel) => { channel }
        Err(e) => {
            error!(target: &http_settings.name, "Cannot send message due to error: {}", e.to_string());
            return Err(error::ErrorServiceUnavailable("Cannot send message."));
        }
    };

    match channel.send(&body).await {
        Ok(_) => {}
        Err(e) => {
            error!(target: &http_settings.name, "Cannot send message due to error: {}", e.to_string());
            return Err(error::ErrorServiceUnavailable("Cannot send message."));
        }
    }

    Ok(HttpResponse::Ok().finish())
}
