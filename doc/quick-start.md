# Quick start

The easiest way to test the flow is with [Docker Compose](https://docs.docker.com/compose/), so make sure that you have
installed both [Docker](https://docs.docker.com/get-docker/)
and [Docker Compose](https://docs.docker.com/compose/install/). The docker image is building and running the required
services. Run `docker-compose up` to start all services, plus generate test messages for 1 minute. If you change the
code, you can always trigger a new build for the services by calling `docker-compose up --build`.

Once the service is up and running, messages can be monitored via RabbitMQ Management
Console ([http://localhost:15672](http://localhost:15672)). Default user and password are `guest/guest`.

Each service info can be retrieved with the `/version` endpoint:

* **amqp-producer** - http://localhost:8080/version
* **amqp-consumer** - http://localhost:8081/version
* **http-sink** - http://localhost:8082/version

Messages can be sent to the producer with a POST request to http://localhost:8080/send