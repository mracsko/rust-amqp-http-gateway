pub mod bin_utils {
    use std::collections::HashMap;

    use lapin::{Connection, ConnectionProperties};
    use log::info;

    use crate::amqp;
    use crate::rest::HttpSettings;
    use crate::version::create_version;

    pub struct AmqpSettings {
        pub queue: String,
    }

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

    pub async fn init_amqp_conn() -> Connection {
        let addr = std::env::var("AMQP_ADDR")
            .unwrap_or_else(|_| "amqp://guest:guest@rabbitmq:5672/%2f".into());
        info!(target: "main", "Value for 'AMQP_ADDR' was read");

        let conn = Connection::connect(&addr, ConnectionProperties::default())
            .await
            .expect("Cannot connect to AMQP host");

        info!(target: "main", "Connected to AMQP");

        conn
    }

    pub async fn init_amqp_conn_and_queue() -> (Connection, AmqpSettings) {
        let queue = std::env::var("AMQP_QUEUE").unwrap_or_else(|_| "main".into());
        info!(target: "main", "Value for 'AMQP_QUEUE': {queue}");

        let conn = init_amqp_conn().await;

        amqp::declare_queue(&conn, &queue).await;

        info!(target: "main", "Declared queue: '{queue}'");

        (conn, AmqpSettings { queue })
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
    use lapin::options::QueueDeclareOptions;
    use lapin::types::FieldTable;
    use lapin::Connection;

    pub async fn declare_queue(conn: &Connection, queue_name: &str) {
        let channel = conn
            .create_channel()
            .await
            .expect("Cannot create AMQP channel");
        channel
            .queue_declare(
                queue_name,
                QueueDeclareOptions::default(),
                FieldTable::default(),
            )
            .await
            .expect("Cannot declare AMQP queue");
    }
}
