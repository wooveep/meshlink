FROM golang:1.22 AS builder
WORKDIR /src
COPY . .
RUN go build -o /out/managementd ./cmd/managementd && \
    go build -o /out/signald ./cmd/signald && \
    go build -o /out/relayd ./cmd/relayd

FROM debian:bookworm-slim
WORKDIR /app
COPY --from=builder /out/managementd /usr/local/bin/managementd
COPY --from=builder /out/signald /usr/local/bin/signald
COPY --from=builder /out/relayd /usr/local/bin/relayd
CMD ["managementd"]
