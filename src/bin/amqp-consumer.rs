use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use actix_web::{App, HttpServer, web};
use bb8::Pool;
use bb8_lapin::LapinConnectionManager;
use futures::StreamExt;
use lapin::{Connection, Consumer};
use lapin::options::{BasicAckOptions, BasicConsumeOptions, BasicRejectOptions};
use lapin::types::FieldTable;
use log::info;
use reqwest::Client;
use serde_json::to_string;

use amqp_service::amqp::AmqpConnectionPool;
use amqp_service::bin_utils::{init_amqp_conn_pool, init_http_settings, init_logging, init_version, read_positive_param};
use amqp_service::rest;
use amqp_service::rest::{create_named_worker, HttpSettings};

const CARGO_BIN_NAME: &'static str = env!("CARGO_BIN_NAME");

#[derive(Clone)]
struct WebhookSettings {
    webhook_addr: String,
    // True if post is used, false if put.
    webhook_method_is_post: bool,
}

#[tokio::main]
async fn main() {
    let start = Instant::now();
    init_logging();

    info!(target: "main", "Service '{CARGO_BIN_NAME}' is starting...");

    let http_settings = init_http_settings(num_cpus::get_physical());
    let amqp_workers = read_positive_param("AMQP_WORKERS", num_cpus::get_physical());
    let amqp_pool = init_amqp_conn_pool().await;
    let webhook_settings = init_webhook_settings();
    let log_per_request = init_amqp_log_per_request();
    let version_string = init_version_string(
        &http_settings,
        &amqp_pool,
        amqp_workers,
        log_per_request,
    );

    register_consumer(
        &amqp_pool,
        amqp_workers,
        &webhook_settings,
        log_per_request,
    )
        .await;

    info!(target: "main", "Binding HTTP server to '{}' on {} thread(s).", &http_settings.http_addr, http_settings.http_workers);

    let worker_counter = Arc::new(Mutex::new(0));
    let server = HttpServer::new(move || {
        let _name = create_named_worker(&worker_counter, "consumer-http", &start);

        App::new()
            .app_data(web::Data::new(rest::VersionJson {
                json: version_string.clone(),
            }))
            .service(rest::version)
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

async fn register_consumer(
    amqp_pool: &AmqpConnectionPool,
    amqp_workers: usize,
    webhook_settings: &WebhookSettings,
    log_threshold: u64,
) {
    let webhook_method = match webhook_settings.webhook_method_is_post {
        true => "POST",
        false => "PUT",
    };
    info!(target: "main","Starting {} consumers to send to webhook ({}): {}",amqp_workers, webhook_method, webhook_settings.webhook_addr);

    let client = reqwest::Client::new();

    let use_post = webhook_settings.webhook_method_is_post;
    for i in 0..amqp_workers {
        let pool = amqp_pool.clone();
        let webhook_addr = webhook_settings.webhook_addr.clone();
        let client = client.clone();
        let queue = amqp_pool.queue_name.to_string();
        let worker_name = format!("consumer-amqp-{}", i + 1);
        tokio::spawn(async move {
            create_and_run_consumer(
                &amqp_pool,
                &queue,
                webhook_addr,
                use_post,
                client,
                worker_name,
                log_threshold,
            )
                .await
        });
    }
}

async fn create_and_run_consumer(
    pool: &Pool<LapinConnectionManager>,
    queue: &str,
    webhook_addr: String,
    use_post: bool,
    client: Client,
    worker_name: String,
    log_threshold: u64,
) {
    let conn = pool.get().await.unwrap();
    let channel = &conn
        .create_channel()
        .await
        .expect("Cannot create AMQP channel.");
    let mut consumer = channel
        .basic_consume(
            queue,
            "consumer",
            BasicConsumeOptions::default(),
            FieldTable::default(),
        )
        .await
        .expect("Cannot create AMQP consumer.");

    info!(
        target: &worker_name,
        "Consumer thread started, logging new line for each {log_threshold} messages..."
    );
    let mut i: u64 = 0;
    let mut was_fail = false;
    let webhook_addr = webhook_addr;
    while let Some(delivery) = consumer.next().await {
        let delivery = delivery.expect("error in consumer");
        let request = if use_post {
            client.post(&webhook_addr)
        } else {
            client.put(&webhook_addr)
        };
        let response = request.body(delivery.data.clone()).send().await;
        match response {
            Ok(_) => {
                delivery.ack(BasicAckOptions::default()).await.expect("ack");
            }
            Err(_) => {
                was_fail = true;
                delivery
                    .reject(BasicRejectOptions::default())
                    .await
                    .expect("reject");
            }
        }
        if log_threshold != 0 && i % log_threshold == 0 {
            if was_fail {
                info!(target: &worker_name, "Processing messages with errors...");
            } else {
                info!(target: &worker_name, "Processing messages...");
            }
            was_fail = false;
        }
        i += 1;
    }
}

fn init_amqp_log_per_request() -> u64 {
    let log_per_request = std::env::var("AMQP_LOG_PER_REQUEST")
        .unwrap_or_else(|_| "0".into())
        .parse::<u64>()
        .expect(
            "Cannot parse 'AMQP_LOG_PER_REQUEST' environment variable. It needs to be a non-negative number.",
        );

    info!(target: "main", "Value for 'AMQP_LOG_PER_REQUEST': {log_per_request}");
    log_per_request
}

fn init_webhook_settings() -> WebhookSettings {
    let webhook_addr = std::env::var("WEBHOOK_ADDR")
        .expect("Webhook address is a mandatory environment variable, pleas set 'WEBHOOK_ADDR'.");
    let webhook_method = std::env::var("WEBHOOK_METHOD").unwrap_or_else(|_| "POST".into());
    let webhook_method_is_post = match webhook_method.to_uppercase().as_str() {
        "POST" => true,
        "PUT" => false,
        _ => panic!(
            "Only 'POST' or 'PUT' is valid value for 'WEBHOOK_METHOD', invalid value is set: '{webhook_method}'"
        ),
    };

    WebhookSettings {
        webhook_addr,
        webhook_method_is_post,
    }
}

fn init_version_string(
    http_settings: &HttpSettings,
    amqp_pool: &AmqpConnectionPool,
    amqp_workers: usize,
    log_per_request: u64,
) -> String {
    let additional_params: HashMap<String, String> = HashMap::from([
        ("cores".to_string(), num_cpus::get_physical().to_string()),
        (
            "http_workers".to_string(),
            http_settings.http_workers.to_string(),
        ),
        ("amqp_queue".to_string(), amqp_pool.queue_name.to_string()),
        ("amqp_workers".to_string(), amqp_workers.to_string()),
        (
            "amqp_log_per_request".to_string(),
            log_per_request.to_string(),
        ),
    ]);
    let version_string = init_version(&CARGO_BIN_NAME, additional_params);
    version_string
}
