pub mod bin_utils {
    use std::collections::HashMap;
    use std::time::Duration;

    use bb8_lapin::LapinConnectionManager;
    use lapin::ConnectionProperties;
    use log::info;

    use crate::amqp::AmqpConnectionPool;
    use crate::rest::HttpSettings;
    use crate::version::create_version;

    pub struct Properties {
        pub log_target: String,
        pub prefix: Option<String>,
    }

    impl Properties {
        pub fn new(log_target: &str, prefix: &str) -> Self {
            Properties {
                log_target: log_target.to_string(),
                prefix: Some(prefix.to_string()),
            }
        }

        pub fn new_no_prefix(log_target: &str) -> Self {
            Properties {
                log_target: log_target.to_string(),
                prefix: None,
            }
        }

        pub fn prop_name(&self, name: &str) -> String {
            match &self.prefix {
                None => name.to_string(),
                Some(prefix) => {
                    format!("{}_{}", prefix, name)
                }
            }
        }

        pub fn init_logging(&self) {
            match std::env::var(self.prop_name("RUST_LOG")) {
                Ok(log_level) => std::env::set_var("RUST_LOG", log_level),
                Err(_) => std::env::set_var("RUST_LOG", "info"),
            }

            tracing_subscriber::fmt::init();
        }

        pub fn init_version(
            &self,
            bin_name: &str,
            additional_params: HashMap<String, String>,
        ) -> String {
            let key = self.prop_name("APP_DETAILED_VERSION");
            let detailed_version = std::env::var(&key)
                .unwrap_or_else(|_| "false".into())
                .parse::<bool>()
                .expect(&format!(
                    "Cannot parse '{key}' environment variable. It needs to be a valid boolean."
                ));
            info!(target: &self.log_target, "Value for '{key}': {detailed_version}");

            let details = if detailed_version {
                additional_params
            } else {
                HashMap::new()
            };

            let version_string = serde_json::to_string(&create_version(bin_name, details))
                .expect("Cannot serialize version ");

            info!(target: &self.log_target, "Version: {version_string}");

            version_string
        }

        pub fn init_http_settings_with_core_count(&self) -> HttpSettings {
            self.init_http_settings(num_cpus::get_physical())
        }

        pub fn init_http_settings(&self, default_core_count: usize) -> HttpSettings {
            let key = self.prop_name("HTTP_ADDR");
            let http_addr = std::env::var(&key).unwrap_or_else(|_| "127.0.0.1:8080".into());
            info!(target: &self.log_target, "Value for '{key}': {http_addr}");

            let http_workers = self.read_positive_param("HTTP_WORKERS", default_core_count);

            HttpSettings {
                http_addr,
                http_workers,
            }
        }

        pub async fn init_amqp_conn_pool(&self) -> AmqpConnectionPool {
            let key = self.prop_name("AMQP_ADDR");
            let addr = std::env::var(&key)
                .unwrap_or_else(|_| "amqp://guest:guest@rabbitmq:5672/%2f".into());
            info!(target: &self.log_target, "Value for '{key}' was read");

            let key = self.prop_name("AMQP_QUEUE");
            let queue = std::env::var(&key).unwrap_or_else(|_| "main".into());
            info!(target: &self.log_target, "Value for '{key}': {queue}");

            let pool_size = self.read_positive_param("AMQP_POOL_SIZE", 1);
            let retry_in_millis = self.read_positive_param("AMQP_RETRY_IN_MILLIS", 10_000);

            let manager = LapinConnectionManager::new(&addr, ConnectionProperties::default());
            let pool = bb8::Pool::builder()
                .max_size(pool_size as u32)
                .build(manager)
                .await
                .expect("Cannot create AMQP connection pool");

            AmqpConnectionPool {
                log_target: self.log_target.clone(),
                pool,
                queue_name: queue,
                sleep_on_fail: Duration::from_millis(retry_in_millis as u64),
            }
        }

        pub fn read_positive_param(
            &self,
            env_var_name: &str,
            default_value: usize,
        ) -> usize {
            let key = self.prop_name(env_var_name);
            let result = std::env::var(&key)
                .unwrap_or_else(|_| default_value.to_string())
                .parse::<usize>()
                .expect("Cannot parse '{key}' environment variable. It needs to be a non negative integer.");
            if result < 1 {
                panic!("Invalid value set for '{key}' environment variable. It needs to be a positive number.");
            }

            info!(target: &self.log_target, "Value for '{key}': {result}");

            result
        }
    }
}

pub mod version {
    use std::collections::HashMap;

    use serde::Serialize;

    pub const APP_VERSION: &str = env!("APP_VERSION");
    pub const CARGO_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
    pub const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");
    pub const GIT_HASH: &str = env!("GIT_HASH");

    #[derive(Serialize, Debug)]
    pub struct Version<'a> {
        pub version: &'a str,
        pub package_version: &'a str,
        pub package_name: &'a str,
        pub binary_name: &'a str,
        pub git_hash: &'a str,
        pub additional_params: HashMap<String, String>,
    }

    pub fn create_version(bin_name: &str, additional_params: HashMap<String, String>) -> Version {
        Version {
            version: APP_VERSION,
            package_version: CARGO_PKG_VERSION,
            package_name: CARGO_PKG_NAME,
            binary_name: bin_name,
            git_hash: GIT_HASH,
            additional_params,
        }
    }
}

pub mod rest {
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use actix_web::{get, web};
    use log::info;

    pub fn create_named_worker(
        worker_counter: &Arc<Mutex<i32>>,
        name_prefix: &str,
        start: &Instant,
    ) -> String {
        let name = format!("{}-{}", name_prefix, {
            let mut x = worker_counter
                .lock()
                .expect("Illegal state, cannot get worker counter");
            *x += 1;
            *x
        });

        info!(
            target: &name,
            "Creating new HTTP thread, inited in {:?}",
            start.elapsed()
        );

        name
    }

    pub struct VersionJson {
        pub json: String,
    }

    pub struct HttpSettings {
        pub http_addr: String,
        pub http_workers: usize,
    }

    #[get("/version")]
    pub async fn version(data: web::Data<VersionJson>) -> String {
        data.json.clone()
    }
}

pub mod amqp {
    use std::error::Error;
    use std::time::Duration;

    use bb8::{Pool, PooledConnection};
    use bb8_lapin::LapinConnectionManager;
    use futures::StreamExt;
    use lapin::{BasicProperties, Channel, Consumer};
    use lapin::message::Delivery;
    use lapin::options::{BasicConsumeOptions, BasicPublishOptions, QueueDeclareOptions};
    use lapin::types::FieldTable;
    use log::{error, info};
    use tokio::time::sleep;

    #[derive(Clone)]
    pub struct AmqpConnectionPool {
        pub log_target: String,
        pub pool: Pool<LapinConnectionManager>,
        pub queue_name: String,
        pub sleep_on_fail: Duration,
    }

    impl AmqpConnectionPool {
        pub fn clone_with_log_target(&self, log_target: &str) -> Self {
            AmqpConnectionPool {
                log_target: log_target.to_owned(),
                pool: self.pool.clone(),
                queue_name: self.queue_name.clone(),
                sleep_on_fail: self.sleep_on_fail,
            }
        }

        pub fn new_wrapper(&self, log_target: &str) -> AmqpChannelWrapper {
            AmqpChannelWrapper::new(self, log_target)
        }

        pub async fn get_connection(
            &self,
        ) -> Result<PooledConnection<'_, LapinConnectionManager>, Box<dyn Error + Send + Sync>> {
            let connection = self.pool.get().await;
            match connection {
                Err(e) => {
                    error!(
                        target: &self.log_target,
                        "Cannot connect to AMQP. Error: {}",
                        e.to_string()
                    );
                    Err(Box::new(e))
                }
                Ok(x) => Ok(x),
            }
        }

        pub async fn get_channel(&self) -> Result<Channel, Box<dyn Error + Send + Sync>> {
            let conn = self.get_connection().await?;
            match conn.create_channel().await {
                Err(e) => {
                    error!(
                        target: &self.log_target,
                        "Cannot create AMQP channel. Error: {}",
                        e.to_string()
                    );
                    Err(Box::new(e))
                }
                Ok(x) => Ok(x),
            }
        }

        pub async fn get_channel_and_declare_queue(&self) -> Result<Channel, Box<dyn Error + Send + Sync>> {
            let channel = self.get_channel().await?;
            match channel
                .queue_declare(
                    &self.queue_name,
                    QueueDeclareOptions::default(),
                    FieldTable::default(),
                )
                .await
            {
                Ok(queue) => {
                    info!(
                        target: &self.log_target,
                        "Queue '{}' is declared, current consumer count is '{}' and current message count is '{}'",
                        self.queue_name,
                        queue.consumer_count(),
                        queue.message_count(),
                    );
                    Ok(channel)
                }
                Err(e) => {
                    error!(
                        target: &self.log_target,
                        "Cannot declare AMQP queue '{}'. Error: {}",
                        self.queue_name,
                        e.to_string()
                    );
                    Err(Box::new(e))
                }
            }
        }
    }

    pub struct AmqpChannelWrapper {
        pool: AmqpConnectionPool,
        channel: Option<Channel>,
    }

    impl AmqpChannelWrapper {
        fn new(pool: &AmqpConnectionPool, log_target: &str) -> Self {
            AmqpChannelWrapper {
                pool: pool.clone_with_log_target(log_target),
                channel: None,
            }
        }

        async fn get_or_create_channel(&mut self) -> Result<&Channel, Box<dyn Error + Send + Sync>> {
            match self.channel {
                None => self.channel = Some(self.pool.get_channel_and_declare_queue().await?),
                Some(_) => {}
            }

            Ok(self.channel.as_ref().expect("At this point value must be available, otherwise we returned with an error already"))
        }

        fn reset_channel(&mut self) {
            self.channel = None;
        }

        async fn handle_error_and_wait(&mut self, func_type: &str, e: &str) {
            error!(target: &self.pool.log_target, "Failed to {func_type} message. Error: {}",e);
            info!(target: &self.pool.log_target, "Retrying AMQP {func_type} in {:?}",self.pool.sleep_on_fail);
            sleep(self.pool.sleep_on_fail).await;
        }

        async fn basic_publish(&mut self, message: &[u8]) -> Result<(), Box<dyn Error + Send + Sync>> {
            //TODO check why
            let queue = self.pool.queue_name.clone();
            let channel = self.get_or_create_channel().await?;
            channel.basic_publish("", &queue, BasicPublishOptions::default(), message, BasicProperties::default())
                .await?.await?;
            Ok(())
        }

        pub async fn send(&mut self, message: &[u8]) -> Result<(), Box<dyn Error>> {
            match self.basic_publish(message).await {
                Ok(_) => { Ok(()) }
                Err(e) => {
                    self.reset_channel();
                    Err(e)
                }
            }
        }

        pub async fn send_retry(&mut self, message: &[u8]) {
            loop {
                match self.send(message).await {
                    Ok(_) => {
                        return;
                    }
                    Err(e) => {
                        self.handle_error_and_wait("send", &e.to_string()).await;
                    }
                }
            }
        }

        pub fn create_consumer(self) -> AmqpConsumerWrapper {
            AmqpConsumerWrapper::new(self)
        }
    }

    pub struct AmqpConsumerWrapper {
        channel: AmqpChannelWrapper,
        consumer: Option<Consumer>,
    }

    impl AmqpConsumerWrapper {
        fn new(amqp_channel: AmqpChannelWrapper) -> AmqpConsumerWrapper {
            AmqpConsumerWrapper {
                channel: amqp_channel,
                consumer: None,
            }
        }

        fn reset_consumer(&mut self) {
            self.channel.reset_channel();
            self.consumer = None;
        }

        async fn get_or_create_consumer(&mut self) -> Result<&mut Consumer, Box<dyn Error + Send + Sync>> {
            match self.consumer {
                None => self.consumer = {
                    let queue = self.channel.pool.queue_name.clone();
                    let log_target = self.channel.pool.log_target.clone();

                    let result = Some(self.channel.get_or_create_channel().await?
                        .basic_consume(
                            &queue,
                            &log_target,
                            BasicConsumeOptions::default(),
                            FieldTable::default(),
                        )
                        .await?);

                    info!(target: &log_target, "Consumer registered");

                    result
                },
                Some(_) => {}
            }

            Ok(self.consumer.as_mut().expect("At this point value must be available, otherwise we returned with an error already"))
        }

        async fn next_message(&mut self) -> Result<Option<Delivery>, Box<dyn Error + Send + Sync>> {
            match self.get_or_create_consumer().await {
                Ok(consumer) => match consumer.next().await {
                    None => Ok(None),
                    Some(delivery) => match delivery {
                        Ok(delivery) => Ok(Some(delivery)),
                        Err(e) => Err(Box::new(e))
                    }
                }
                Err(e) => Err(e)
            }
        }

        pub async fn receive_message(&mut self) -> Result<Option<Delivery>, Box<dyn Error + Send + Sync>> {
            match self.next_message().await {
                Ok(delivery) => {
                    Ok(delivery)
                }
                Err(e) => {
                    self.reset_consumer();
                    Err(e)
                }
            }
        }

        pub async fn receive_message_retry(&mut self) -> Option<Delivery> {
            loop {
                match self.receive_message().await {
                    Ok(msg) => return msg,
                    Err(e) => {
                        self.reset_consumer();
                        self.channel.handle_error_and_wait("send", &e.to_string()).await;
                    }
                };
            }
        }
    }
}