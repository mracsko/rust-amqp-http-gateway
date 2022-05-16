use std::collections::HashMap;
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use actix_web::{App, HttpServer, web};
use lapin::message::Delivery;
use lapin::options::{BasicAckOptions, BasicNackOptions, BasicRejectOptions};
use log::{error, info};
use reqwest::RequestBuilder;

use amqp_service::amqp::AmqpConnectionPool;
use amqp_service::bin_utils::Properties;
use amqp_service::rest;
use amqp_service::rest::{create_named_worker, HttpSettings};

const CARGO_BIN_NAME: &'static str = env!("CARGO_BIN_NAME");

#[derive(Clone)]
struct WebhookSettings {
    webhook_addr: String,
    // True if post is used, false if put.
    webhook_method_is_post: bool,
}

struct ConsumerProperties {
    base: Properties,
}

impl ConsumerProperties {
    fn new() -> Self {
        let base = Properties::new("main", "CONS");
        ConsumerProperties { base }
    }

    fn init_version_string(
        &self,
        http_settings: &HttpSettings,
        amqp_pool: &AmqpConnectionPool,
        amqp_workers: usize,
    ) -> String {
        let additional_params: HashMap<String, String> = HashMap::from([
            ("cores".to_string(), num_cpus::get_physical().to_string()),
            (
                "http_workers".to_string(),
                http_settings.http_workers.to_string(),
            ),
            ("amqp_queue".to_string(), amqp_pool.queue_name.to_string()),
            ("amqp_workers".to_string(), amqp_workers.to_string()),
        ]);
        self.base.init_version(CARGO_BIN_NAME, additional_params)
    }

    fn init_webhook_settings(&self) -> WebhookSettings {
        let key = self.base.prop_name("WEBHOOK_ADDR");
        let webhook_addr = std::env::var(&key)
            .expect("Webhook address is a mandatory environment variable, pleas set '{key}'.");

        let key = self.base.prop_name("WEBHOOK_METHOD");
        let webhook_method = std::env::var(&key).unwrap_or_else(|_| "POST".into());
        let webhook_method_is_post = match webhook_method.to_uppercase().as_str() {
            "POST" => true,
            "PUT" => false,
            _ => panic!(
                "Only 'POST' or 'PUT' is valid value for '{key}', invalid value is set: '{webhook_method}'"
            ),
        };

        WebhookSettings {
            webhook_addr,
            webhook_method_is_post,
        }
    }
}

#[tokio::main]
async fn main() {
    let start = Instant::now();

    let properties = ConsumerProperties::new();
    properties.base.init_logging();

    info!(target: &properties.base.log_target, "Service '{CARGO_BIN_NAME}' is starting...");

    let http_settings = properties.base.init_http_settings_with_core_count();
    let amqp_workers = properties
        .base
        .read_positive_param("AMQP_WORKERS", num_cpus::get_physical());
    let amqp_pool = properties.base.init_amqp_conn_pool().await;
    let webhook_settings = properties.init_webhook_settings();
    let version_string =
        properties.init_version_string(&http_settings, &amqp_pool, amqp_workers);

    register_consumer(
        &properties,
        &amqp_pool,
        amqp_workers,
        &webhook_settings,
    )
        .await;

    info!(target: &properties.base.log_target, "Binding HTTP server to '{}' on {} thread(s).", &http_settings.http_addr, http_settings.http_workers);

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

    info!(target: &properties.base.log_target, "Service inited in {:?}", start.elapsed());

    server
        .run()
        .await
        .expect("HTTP server could not be started");
}

async fn register_consumer(
    properties: &ConsumerProperties,
    amqp_pool: &AmqpConnectionPool,
    amqp_workers: usize,
    webhook_settings: &WebhookSettings,
) {
    let webhook_method = match webhook_settings.webhook_method_is_post {
        true => "POST",
        false => "PUT",
    };
    info!(target: &properties.base.log_target,"Starting {} consumers to send to webhook ({}): {}",amqp_workers, webhook_method, webhook_settings.webhook_addr);

    let use_post = webhook_settings.webhook_method_is_post;
    for i in 0..amqp_workers {
        let worker_name = format!("consumer-amqp-{}", i + 1);
        let webhook_addr = webhook_settings.webhook_addr.clone();
        let amqp_channel = amqp_pool.new_wrapper(&worker_name);

        tokio::spawn(async move {
            let mut consumer = amqp_channel.create_consumer();
            let client = reqwest::Client::new();

            info!(target: &worker_name, "Consumer thread started...");

            while let Some(delivery) = consumer.receive_message_retry().await {
                let request = {
                    if use_post {
                        client.post(&webhook_addr)
                    } else {
                        client.put(&webhook_addr)
                    }
                };
                match process(&delivery, request, &worker_name).await {
                    Ok(_) => {}
                    Err(e) => {
                        error!("Acknowledgement failed. Error: {}",e);
                    }
                }
            }
        });
    }
}

async fn process(delivery: &Delivery, request: RequestBuilder, worker_name: &str) -> Result<(), Box<dyn Error>> {
    let message = delivery.data.clone();
    let response = request.body(message).send().await;
    match response {
        Ok(response) => {
            if response.status().is_client_error() {
                error!(
                        target: &worker_name,
                        "Webhook could not process message. Status code: {}",
                        response.status().as_u16()
                    );
                delivery.reject(BasicRejectOptions::default()).await?;
            } else {
                delivery.ack(BasicAckOptions::default()).await?;
            }
        }
        Err(e) => {
            error!(
                    target: &worker_name,
                    "Cannot send message to webhook. Error: {}", e
                );
            delivery.nack(BasicNackOptions::default()).await?;
        }
    };
    Ok(())
}