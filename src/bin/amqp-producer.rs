use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use actix_web::{error, post, web, App, Error, HttpResponse, HttpServer};
use futures_lite::StreamExt;
use lapin::options::BasicPublishOptions;
use lapin::{BasicProperties, Channel};
use log::{error, info};

use amqp_service::bin_utils::{
    init_amqp_conn_and_queue, init_http_settings, init_logging, init_version, AmqpSettings,
};
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
    let (conn, amqp_settings) = init_amqp_conn_and_queue().await;
    let version_string = init_version_string(&http_settings, &amqp_settings);

    info!(target: "main", "Binding HTTP server to '{}' on {} thread(s).", &http_settings.http_addr, http_settings.http_workers);

    let conn = Arc::new(Mutex::new(conn));
    let worker_counter = Arc::new(Mutex::new(0));
    let server = HttpServer::new(move || {
        let name = create_named_worker(&worker_counter, "producer", &start);

        App::new()
            .app_data(web::Data::new(rest::VersionJson {
                json: version_string.clone(),
            }))
            .app_data(web::Data::new(futures::executor::block_on(async {
                conn.lock()
                    .expect("Illegal state, cannot acquire lock")
                    .create_channel()
                    .await
                    .expect("Cannot create AMQP channel.")
            })))
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

fn init_version_string(http_settings: &HttpSettings, amqp_settings: &AmqpSettings) -> String {
    let additional_params: HashMap<String, String> = HashMap::from([
        ("cores".to_string(), num_cpus::get_physical().to_string()),
        (
            "http_workers".to_string(),
            http_settings.http_workers.to_string(),
        ),
        ("amqp_queue".to_string(), amqp_settings.queue.to_string()),
    ]);
    let version_string = init_version(&CARGO_BIN_NAME, additional_params);
    version_string
}

#[post("/send")]
async fn send(
    mut payload: web::Payload,
    channel: web::Data<Channel>,
    settings: web::Data<HttpWorkerSettings>,
) -> Result<HttpResponse, Error> {
    let mut body = web::BytesMut::new();
    while let Some(chunk) = payload.next().await {
        let chunk = chunk?;
        if (body.len() + chunk.len()) > MAX_SIZE {
            return Err(error::ErrorBadRequest("overflow"));
        }
        body.extend_from_slice(&chunk);
    }

    match channel
        .basic_publish(
            "",
            "main",
            BasicPublishOptions::default(),
            &body,
            BasicProperties::default(),
        )
        .await
    {
        Ok(conf) => match conf.await {
            Ok(_) => {}
            Err(e) => {
                error!(target: &settings.name, "Cannot send message due to error: {}", e.to_string());
                return Err(error::ErrorServiceUnavailable("Cannot send message."));
            }
        },
        Err(e) => {
            error!(target: &settings.name, "Cannot send message due to error: {}", e.to_string());
            return Err(error::ErrorServiceUnavailable("Cannot send message."));
        }
    }

    Ok(HttpResponse::Ok().finish())
}
