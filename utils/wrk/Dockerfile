FROM debian:bullseye-slim as builder
RUN apt-get update -y && apt-get upgrade -y && apt-get install -y git zip build-essential wait-for-it && rm -rf /var/lib/apt/lists/*

WORKDIR /tmp
RUN git clone https://github.com/wg/wrk

WORKDIR /tmp/wrk
RUN make

FROM debian:bullseye-slim
RUN apt-get update -y && apt-get upgrade -y && apt-get install -y netbase wait-for-it && rm -rf /var/lib/apt/lists/*
COPY --from=builder /tmp/wrk/wrk /usr/local/bin/
COPY  ./scripts ./scripts
