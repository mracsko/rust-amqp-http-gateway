# Services

![Components](components.png)

The default setup starts a RabbitMQ service, generates messages with wrk (message-generator) and calls the HTTP endpoint
on amqp-producer. The producer publish the messages to RabbitMQ.

RabbitMQ delivers the messages to the amqp-consumer, that calls a webhook (http-sink) to simulate message processing.

* **Message generator (wrk):** Service using [wrk](https://github.com/wg/wrk) to generate HTTP requests to publish
  messages with content in [utils/wrk/scripts/post-json.lua](utils/wrk/scripts/post-json.lua).
* **Message producer (amqp-producer):** Custom service for receiving HTTP POST requests and publishing the body to
  RabbitMQ queue.
* **RabbitMQ:** [RabbitMQ](https://www.rabbitmq.com/) instance.
* **Message consumer (amqp-consumer):** Custom service for consuming RabbitMQ messages and calling a webhook.
* **Http Sink (http-sink):** Custom service for simulating a webhook. Receives and ignores HTTP requests.

## Environment variables

**Generic environment variables used by all custom services (amqp-producer, amqp-consumer, http-sink):**

* **RUST_LOG:** Setting the log level the application. Directly
  the [tracing_subscriber](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/) crate is used, that in the
  background is using [env_logger](https://docs.rs/env_logger/0.9.0/env_logger/) crate. For details settings check the
  documentation of [env_logger](https://docs.rs/env_logger/0.9.0/env_logger/) crate. _Default log level: `info`_
* **APP_DETAILED_VERSION:** Enabling or disabling detailed logging. If enabled, additional parameters are provided by
  the `/version` endpoint. Needs to be a valid boolean value. _Default: `false`_
* **HTTP_ADDR:** Bind address for Actix Web, in `host:port` format. Further details are available
  in [Actix Web Documentation](https://docs.rs/actix-web/2.0.0/actix_web/struct.HttpServer.html#method.bind). _Default
  value: `127.0.0.1:8080`_
* **HTTP_WORKERS:** Number of worker threads for Actix Web. Needs to be a positive integer. Further details are
  available
  in [Actix Web Documentation](https://docs.rs/actix-web/2.0.0/actix_web/struct.HttpServer.html#method.workers).

## Third party services

### message-generator service

Dockerized [wrk](https://github.com/wg/wrk), with the possibility of adding custom scripts during compile time. For more
details check  [utils/wrk/Dockerfile](utils/wrk/Dockerfile).

Scripts are available in [utils/wrk/scripts/](utils/wrk/scripts/). A new docker build is required if new scripts are
added.

### rabbitmq service

Using official [RabbitMQ Docker Image](https://hub.docker.com/_/rabbitmq) to start RabbitMQ with management console.
Documentation can be found on [RabbitMQ Website](https://www.rabbitmq.com/).

## Custom services

### amqp-producer service

Custom service for receiving HTTP POST requests and publishing the body to RabbitMQ queue.

#### Environment variables

Environment variables that are available beside the previously mentioned generic ones.

* **HTTP_WORKERS:** _Default value is the number of CPU cores._
* **AMQP_ADDR:** Connection string for AMQP (RabbitMQ) service. _Default value: `amqp://guest:guest@rabbitmq:5672/%2f`_
* **AMQP_QUEUE:** The name of the AMQP queue that is used. If it does not exist it is created. _Default value: `main`_

#### Endpoints

* **[GET] /version** provides version information about the running process (cargo build info, git hash, additional
  parameters). Additional parameters are only provided if environment variable _APP_DETAILED_VERSION_ is set.
* **[POST] /send** puts the request body as a message to the AMQP queue.

### amqp-consumer service

Custom service for consuming RabbitMQ messages and calling a webhook.

#### Environment variables

Environment variables that are available beside the previously mentioned generic ones.

* **HTTP_WORKERS:** _Default value is `1`_
* **AMQP_WORKERS:** Number of worker threads for spawned for listening on messages. Needs to be a positive integer.
  _Default value is the number of CPU cores._
* **AMQP_ADDR:** Connection string for AMQP (RabbitMQ) service. _Default value: `amqp://guest:guest@rabbitmq:5672/%2f`_
* **AMQP_QUEUE:** The name of the AMQP queue that is used. If it does not exist it is created. _Default value: `main`_
* **AMQP_LOG_PER_REQUEST:** Log a static text after the defined amount of messages received on an AMQP worker thread.
  Needs to be non-negative integer. Set to `0` to disable this log. _Default value: `0`_

#### Endpoints

* **[GET] /version** provides version information about the running process (cargo build info, git hash, additional
  parameters). Additional parameters are only provided if environment variable _APP_DETAILED_VERSION_ is set.

### http-sink service

Custom service for simulating a webhook. Receives and ignores HTTP requests.

#### Environment variables

Environment variables that are available beside the previously mentioned generic ones.

* **HTTP_WORKERS:** _Default value is the number of CPU cores._
* **HTTP_LOG_PER_REQUEST:** Log a static text after the defined amount of messages received on a HTTP worker thread.
  Needs to be non-negative integer. Set to `0` to disable this log. _Default value: `0`_

#### Endpoints

* **[GET] /version** provides version information about the running process (cargo build info, git hash, additional
  parameters). Additional parameters are only provided if environment variable _APP_DETAILED_VERSION_ is set. TBD

## Docker Compose

The provided [docker-compose.yml](docker-compose.yml) file sets up all services to communicate with each other. All
custom services are set up to provide detailed version information, and logging thresholds are set to 10000.

[Wrk](https://github.com/wg/wrk) is running for 60 seconds, on 2 threads, 10 connections. Docker source for wrk can be
found in [utils/wrk/Dockerfile](utils/wrk/Dockerfile). Message body is defined
in [utils/wrk/scripts/post-json.lua](utils/wrk/scripts/post-json.lua)

Services are waiting for available endpoints (startup and initialization)
with [wait-fo-it](https://github.com/vishnubob/wait-for-it).