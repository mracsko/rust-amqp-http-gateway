FROM amd64/rust:1.60.0-bullseye as builder

WORKDIR /usr/src/amqp-service

COPY . .

RUN cargo install --path .

FROM debian:bullseye-slim
RUN apt-get update -y && apt-get upgrade -y && apt-get install -y wait-for-it && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/cargo/bin/amqp-* /usr/local/bin/
COPY --from=builder /usr/local/cargo/bin/http-sink /usr/local/bin/

EXPOSE 8080
