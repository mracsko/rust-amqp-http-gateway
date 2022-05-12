pub mod bin_utils {
    use std::collections::HashMap;
    use std::time::Duration;

    use bb8::{Pool, PooledConnection, RunError};
    use bb8_lapin::LapinConnectionManager;
    use lapin::{Connection, ConnectionProperties, Error};
    use log::{error, info};

    use crate::amqp;
    use crate::amqp::AmqpConnectionPool;
    use crate::rest::HttpSettings;
    use crate::version::create_version;

    pub fn init_logging() {
        if std::env::var("RUST_LOG").is_err() {
            std::env::set_var("RUST_LOG", "info");
        }
        tracing_subscriber::fmt::init();
    }

    pub fn init_version(
        bin_name: &'static str,
        additional_params: HashMap<String, String>,
    ) -> String {
        let detailed_version = std::env::var("APP_DETAILED_VERSION").unwrap_or_else(|_| "false".into()).parse::<bool>()
            .expect("Cannot parse 'APP_DETAILED_VERSION' environment variable. It needs to be a valid boolean.");
        info!(target: "main", "Value for 'APP_DETAILED_VERSION': {detailed_version}");

        let version_string = if detailed_version {
            serde_json::to_string(&create_version(&bin_name, additional_params))
                .expect("Cannot serialize version ")
        } else {
            serde_json::to_string(&create_version(&bin_name, HashMap::new()))
                .expect("Cannot serialize version ")
        };

        info!(target: "main", "Version: {version_string}");

        version_string
    }

    pub async fn init_amqp_conn_pool() -> AmqpConnectionPool {
        let addr = std::env::var("AMQP_ADDR")
            .unwrap_or_else(|_| "amqp://guest:guest@rabbitmq:5672/%2f".into());
        info!(target: "main", "Value for 'AMQP_ADDR' was read");

        let queue = std::env::var("AMQP_QUEUE").unwrap_or_else(|_| "main".into());
        info!(target: "main", "Value for 'AMQP_QUEUE': {queue}");

        let pool_size = read_positive_param("AMQP_POOL_SIZE", 1);
        let retry_in_millis = read_positive_param("AMQP_RETRY_IN_MILLIS", 10_000);

        let manager = LapinConnectionManager::new(&addr, ConnectionProperties::default());
        let pool = bb8::Pool::builder()
            .max_size(pool_size as u32)
            .build(manager)
            .await
            .unwrap();

        AmqpConnectionPool {
            log_target: "main".to_string(),
            pool,
            queue_name: queue,
            sleep_on_fail: Duration::from_millis(retry_in_millis as u64),
        }
    }

    pub fn init_http_settings(default_core_count: usize) -> HttpSettings {
        let http_addr = std::env::var("HTTP_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".into());
        info!(target: "main", "Value for 'HTTP_ADDR': {http_addr}");

        let http_workers = read_positive_param("HTTP_WORKERS", default_core_count);

        HttpSettings {
            http_addr,
            http_workers,
        }
    }

    pub fn read_positive_param(env_var_name: &str, default_value: usize) -> usize {
        let result = std::env::var(env_var_name)
            .unwrap_or_else(|_| default_value.to_string())
            .parse::<usize>()
            .expect("Cannot parse '{env_var_name}' environment variable. It needs to be a non negative integer.");
        if result < 1 {
            panic!("Invalid value set for '{env_var_name}' environment variable. It needs to be a positive number.");
        }
        info!(target: "main", "Value for '{env_var_name}': {result}");

        result
    }
}

pub mod version {
    use std::collections::HashMap;

    use serde::Serialize;

    pub const APP_VERSION: &'static str = env!("APP_VERSION");
    pub const CARGO_PKG_VERSION: &'static str = env!("CARGO_PKG_VERSION");
    pub const CARGO_PKG_NAME: &'static str = env!("CARGO_PKG_NAME");
    pub const GIT_HASH: &'static str = env!("GIT_HASH");

    #[derive(Serialize, Debug)]
    pub struct Version<'a> {
        pub version: &'a str,
        pub package_version: &'a str,
        pub package_name: &'a str,
        pub binary_name: &'a str,
        pub git_hash: &'a str,
        pub additional_params: HashMap<String, String>,
    }

    pub fn create_version(
        bin_name: &'static str,
        additional_params: HashMap<String, String>,
    ) -> Version<'static> {
        Version {
            version: APP_VERSION,
            package_version: CARGO_PKG_VERSION,
            package_name: CARGO_PKG_NAME,
            binary_name: &bin_name,
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
        let version_info = &data.json;
        format!("{version_info}")
    }
}

pub mod amqp {
    use std::future::Future;
    use std::time::Duration;

    use bb8::{Pool, PooledConnection, RunError};
    use bb8_lapin::LapinConnectionManager;
    use lapin::{BasicProperties, Channel, Connection, Error};
    use lapin::options::{BasicPublishOptions, QueueDeclareOptions};
    use lapin::types::FieldTable;
    use log::{error, info};
    use tokio::sync::broadcast::channel;
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
                sleep_on_fail: self.sleep_on_fail.clone(),
            }
        }

        pub fn new_wrapper(&self, log_target: &str) -> AmqpChannelWrapper {
            AmqpChannelWrapper::new(self, log_target)
        }

        pub async fn get_connection(&self) -> Result<PooledConnection<'_, LapinConnectionManager>, Error> {
            let connection = self.pool.get().await;
            match connection {
                Err(e) => {
                    error!(
                        target: &self.log_target,
                        "Cannot connect to AMQP. Error: {}",
                        e.to_string()
                    );
                    //TODO add proper error handling
                    Err(Error::ChannelsLimitReached)
                }
                Ok(x) => Ok(x),
            }
        }

        pub async fn get_channel(&self) -> Result<Channel, Error> {
            let conn = self.get_connection().await?;
            match conn.create_channel().await {
                Err(e) => {
                    error!(
                                target: &self.log_target,
                                "Cannot create AMQP channel. Error: {}",
                                e.to_string()
                            );
                    Err(e)
                }
                x => x,
            }
        }

        pub async fn get_channel_and_declare_queue(&self) -> Result<Channel, Error> {
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
                    Err(e)
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

        async fn get_or_create_channel(&mut self) -> Result<&Channel, Error> {
            match self.channel {
                None => {
                    self.channel = Some(self.pool.get_channel_and_declare_queue().await?)
                }
                Some(_) => {}
            }

            let result = self.channel.as_ref().unwrap();

            Ok(result)
        }

        fn reset_channel(&mut self) {
            self.channel = None;
        }

        async fn basic_publish(&mut self, message: &[u8]) -> Result<(), Error> {
            let queue = &self.pool.queue_name.clone();
            let channel = &self.get_or_create_channel().await?;
            match channel.basic_publish(
                "",
                &queue,
                BasicPublishOptions::default(),
                message,
                BasicProperties::default(),
            ).await {
                Ok(conf) => match conf.await {
                    Ok(_) => { Ok(()) }
                    Err(e) => {
                        error!(target: &self.pool.log_target, "Cannot send message due to confirmation error: {}", e.to_string());
                        self.reset_channel();
                        Err(e)
                    }
                },
                Err(e) => {
                    error!(target: &self.pool.log_target, "Cannot send message due to error: {}", e.to_string());
                    self.reset_channel();
                    Err(e)
                }
            }
        }

        pub async fn send(&mut self, message: &[u8]) -> Result<(), Error> {
            self.basic_publish(message).await?;
            Ok(())
        }

        pub async fn send_retry(&mut self, message: &[u8]) {
            loop {
                let result = self.send(message).await;

                match result {
                    Ok(_) => { return; }
                    Err(_) => {
                        info!(target: &self.pool.log_target, "Retrying AMQP send in {:?}",self.pool.sleep_on_fail);
                        sleep(self.pool.sleep_on_fail).await;
                    }
                }
            }
        }
    }
}
