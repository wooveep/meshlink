# MeshLink 1.0.0 安装部署交付说明

本文用于 `1.0.0` 发布交付，覆盖部署产物、安装顺序、启动检查和验收命令。完整运维说明见 `docs/operations/deployment-management-usage.md`。

## 1. 发布产物

从仓库根目录构建：

```bash
make build-server
make build-client
make package-deb
make package-windows
```

输出目录：

1. Linux 服务端二进制：`dist/bin/linux-amd64/server/`
2. Linux 客户端二进制：`dist/bin/linux-amd64/client/`
3. Debian 安装包：`dist/deb/`
4. Windows 客户端 zip：`dist/windows/x86_64-pc-windows-gnu/`
5. 校验文件：`dist/SHA256SUMS`

`1.0.0` Debian 包名：

```text
meshlink-managementd_1.0.0-1_amd64.deb
meshlink-signald_1.0.0-1_amd64.deb
meshlink-relayd_1.0.0-1_amd64.deb
meshlink-client_1.0.0-1_amd64.deb
```

Windows 包名：

```text
meshlink-client_1.0.0_x86_64-pc-windows-gnu.zip
```

## 2. Linux 服务端安装

在管理节点安装三个服务端包：

```bash
sudo dpkg -i dist/deb/meshlink-managementd_1.0.0-1_amd64.deb
sudo dpkg -i dist/deb/meshlink-signald_1.0.0-1_amd64.deb
sudo dpkg -i dist/deb/meshlink-relayd_1.0.0-1_amd64.deb
```

编辑 `/etc/default/meshlink-managementd.env`：

```dotenv
MESHLINK_MANAGEMENTD_LISTEN=:33073
MESHLINK_MANAGEMENTD_HTTP_LISTEN=:8080
MESHLINK_MANAGEMENTD_BOOTSTRAP_TOKEN=replace-me
MESHLINK_MANAGEMENTD_ADMIN_TOKEN=replace-me-admin-token
MESHLINK_MANAGEMENTD_OVERLAY_CIDR=100.64.0.0/10
MESHLINK_MANAGEMENTD_STATE_DB=/var/lib/meshlink/management.db
MESHLINK_SIGNALD_STATUS_URL=http://127.0.0.1:18080/internal/status/v1/service
MESHLINK_RELAYD_STATUS_URL=http://127.0.0.1:18081/internal/status/v1/service
```

启动管理服务：

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now meshlink-managementd
sudo systemctl status meshlink-managementd --no-pager
```

`signald` 和 `relayd` 当前包内提供二进制和默认环境文件，生产部署可用现有进程管理系统托管。最小启动命令：

```bash
set -a
source /etc/default/meshlink-signald.env
set +a
nohup /usr/bin/signald \
  -listen "${MESHLINK_SIGNALD_LISTEN}" \
  -stun-listen "${MESHLINK_SIGNALD_STUN_LISTEN}" \
  -status-listen 127.0.0.1:18080 \
  -management-addr "${MESHLINK_SIGNALD_MANAGEMENT_ADDR}" \
  -bootstrap-token "${MESHLINK_SIGNALD_BOOTSTRAP_TOKEN}" \
  -heartbeat-timeout "${MESHLINK_SIGNALD_HEARTBEAT_TIMEOUT}" \
  >/var/log/meshlink-signald.log 2>&1 &

set -a
source /etc/default/meshlink-relayd.env
set +a
nohup /usr/bin/relayd \
  -listen "${MESHLINK_RELAYD_LISTEN}" \
  -status-listen 127.0.0.1:18081 \
  -management-addr "${MESHLINK_RELAYD_MANAGEMENT_ADDR}" \
  -bootstrap-token "${MESHLINK_RELAYD_BOOTSTRAP_TOKEN}" \
  -session-ttl "${MESHLINK_RELAYD_SESSION_TTL}" \
  -advertise-host "${MESHLINK_RELAYD_ADVERTISE_HOST}" \
  >/var/log/meshlink-relayd.log 2>&1 &
```

`MESHLINK_RELAYD_ADVERTISE_HOST` 必须配置为客户端真实可达的公网 IP 或 DNS 名称。

## 3. Linux 客户端安装

目标节点需要内核 WireGuard 支持、`iproute2`，并以 root 或等效权限运行客户端。

```bash
sudo dpkg -i dist/deb/meshlink-client_1.0.0-1_amd64.deb
sudoedit /etc/meshlink/client.toml
```

最小配置：

```toml
management_addr = "192.0.2.10:33073"
bootstrap_token = "replace-me"
```

启动：

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now meshlink-client
sudo systemctl status meshlink-client --no-pager
```

## 4. Windows 客户端安装

解压 `dist/windows/x86_64-pc-windows-gnu/meshlink-client_1.0.0_x86_64-pc-windows-gnu.zip`，保持以下文件在同一目录：

```text
meshlinkd.exe
client.example.toml
run-meshlinkd.ps1
tunnel.dll
wireguard.dll
wintun.dll
```

将 `client.example.toml` 复制为 `client.toml`，至少配置：

```toml
management_addr = "192.0.2.10:33073"
bootstrap_token = "replace-me"
interface_name = "MeshLink"
```

用管理员 PowerShell 启动：

```powershell
powershell.exe -ExecutionPolicy Bypass -File .\run-meshlinkd.ps1
```

## 5. 发布验收

本地基础验收：

```bash
make lint
make test
```

Linux VM 全流程验收：

```bash
./tests/nat-lab/create-lab.sh
./tests/nat-lab/run-phase01-02.sh
./tests/nat-lab/run-phase03.sh
./tests/nat-lab/run-phase03-deb.sh
MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase05.sh
MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase06.sh
MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase08-routes.sh
./tests/nat-lab/destroy-lab.sh
```

Windows VM 验收：

```bash
make package-windows
./tests/windows-vm/run-phase08-validation.sh
```

## 6. 上线检查

服务端：

```bash
curl -fsS http://127.0.0.1:8080/admin/ >/dev/null
curl -fsS http://127.0.0.1:18080/internal/status/v1/service
curl -fsS http://127.0.0.1:18081/internal/status/v1/service
```

Linux 客户端：

```bash
sudo ip addr show sdwan0
sudo wg show
ping <peer-overlay-ip>
```

Windows 客户端：

```powershell
Get-Service WireGuardTunnel$MeshLink
Get-NetAdapter MeshLink
Test-Connection <peer-overlay-ip>
```
