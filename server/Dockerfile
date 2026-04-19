FROM node:24-bookworm-slim AS admin-ui-builder
WORKDIR /ui
COPY admin-ui/package.json admin-ui/package-lock.json* ./
RUN if [ -f package-lock.json ]; then npm ci; else npm install; fi
COPY admin-ui/ ./
RUN npm run build

FROM golang:1.22 AS builder
WORKDIR /src
COPY . .
COPY --from=admin-ui-builder /ui/dist /src/internal/adminui/dist
RUN go build -o /out/managementd ./cmd/managementd && \
    go build -o /out/signald ./cmd/signald && \
    go build -o /out/relayd ./cmd/relayd

FROM debian:bookworm-slim
WORKDIR /app
COPY --from=builder /out/managementd /usr/local/bin/managementd
COPY --from=builder /out/signald /usr/local/bin/signald
COPY --from=builder /out/relayd /usr/local/bin/relayd
CMD ["managementd"]
