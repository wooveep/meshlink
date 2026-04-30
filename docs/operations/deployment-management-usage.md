# MeshLink 部署、管理、使用手册

## 1. 适用范围

本文面向当前仓库已落地的交付形态，覆盖：

1. Linux 服务端：`managementd`、`signald`、`relayd`
2. Linux 客户端：`meshlinkd`
3. Windows 客户端：zip 包 + embedded tunnel-service runtime
4. 管理台：`managementd` 提供的 `/admin/`

当前手册基于 `1.0.0` 发布准备状态编写，适用于 README 中描述的 “Linux 完整闭环 + Relay fallback + Route advertisement + Embedded Windows Runtime + Web 管理台” 阶段。

## 2. 组件与职责

### 2.1 服务端组件

1. `managementd`
   负责设备注册、Overlay IPv4 分配、`SyncConfig` 推送、SQLite 状态持久化，以及 Web 管理台 `/admin/` 与 `/api/admin/v1/*`。
2. `signald`
   负责 NAT 打洞协商、STUN、candidate exchange，以及内部状态输出。
3. `relayd`
   负责直连失败后的 relay reservation 和 UDP 转发。

### 2.2 客户端组件

1. `meshlinkd`
   负责向 `managementd` 注册、订阅 peer 视图、维护本地 WireGuard 接口，并在需要时接入 `signald` / `relayd`。
2. Windows embedded runtime
   当前 Windows 路径不依赖外部安装的 WireGuard GUI；包内自带 `tunnel.dll`、`wireguard.dll`、`wintun.dll`。

### 2.3 当前推荐拓扑

首版推荐单管理节点部署：

1. 一台 Linux 主机运行 `managementd`、`signald`、`relayd`
2. Linux 或 Windows 节点运行 `meshlinkd`
3. 管理员通过浏览器访问 `http://<management-host>:8080/admin/`

## 3. 端口与文件约定

### 3.1 默认端口

1. `managementd` gRPC：`33073/tcp`
2. `managementd` 管理台 HTTP：`8080/tcp`
3. `signald` gRPC：`10000/tcp`
4. `signald` STUN：`3479/udp`
5. `relayd` 控制地址：默认配置为 `3478`
   客户端通过 `relay_addr` 访问该地址，请确保对客户端可达
6. `signald` 内部状态：`18080/tcp`
7. `relayd` 内部状态：`18081/tcp`

### 3.2 Linux 安装路径

1. `managementd`：`/usr/bin/managementd`
2. `signald`：`/usr/bin/signald`
3. `relayd`：`/usr/bin/relayd`
4. `meshlinkd`：`/usr/bin/meshlinkd`
5. 管理服务环境文件：`/etc/default/meshlink-managementd.env`
6. 客户端配置：`/etc/meshlink/client.toml`
7. 管理状态库：`/var/lib/meshlink/management.db`

### 3.3 Windows 包内容

Windows zip 包默认包含：

1. `meshlinkd.exe`
2. `client.example.toml`
3. `run-meshlinkd.ps1`
4. `tunnel.dll`
5. `wireguard.dll`
6. `wintun.dll`

## 4. 部署手册

### 4.1 构建交付物

在仓库根目录执行：

```bash
make build-server
make build-client
make package-deb
make package-windows
```

输出位置：

1. Linux 二进制：`dist/bin/linux-amd64/`
2. Debian 包：`dist/deb/`
3. Windows zip 包：`dist/windows/<target>/`

如果 Windows runtime DLL 尚未 stage，可先执行：

```bash
make windows-runtime
make package-windows
```

### 4.2 Linux 服务端部署

#### 4.2.1 安装 Debian 包

在管理节点安装：

```bash
sudo dpkg -i dist/deb/meshlink-managementd_*_amd64.deb
sudo dpkg -i dist/deb/meshlink-signald_*_amd64.deb
sudo dpkg -i dist/deb/meshlink-relayd_*_amd64.deb
```

#### 4.2.2 配置 `managementd`

编辑 `/etc/default/meshlink-managementd.env`，至少修改以下字段：

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

字段说明：

1. `BOOTSTRAP_TOKEN`
   客户端、`signald`、`relayd` 必须与它一致。
2. `ADMIN_TOKEN`
   仅用于浏览器登录管理台。
3. `OVERLAY_CIDR`
   管理面分配给客户端的 Overlay 地址池。
4. `STATE_DB`
   SQLite 状态库；重启后设备状态和审计事件从这里恢复。

#### 4.2.3 启动 `managementd`

仓库已提供 systemd unit：

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now meshlink-managementd
sudo systemctl status meshlink-managementd --no-pager
```

#### 4.2.4 启动 `signald`

当前包内提供二进制和环境文件示例，但没有内置 systemd unit。最小启动示例：

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
```

示例环境文件字段：

```dotenv
MESHLINK_SIGNALD_LISTEN=:10000
MESHLINK_SIGNALD_STUN_LISTEN=:3479
MESHLINK_SIGNALD_MANAGEMENT_ADDR=127.0.0.1:33073
MESHLINK_SIGNALD_BOOTSTRAP_TOKEN=replace-me
MESHLINK_SIGNALD_HEARTBEAT_TIMEOUT=15s
```

#### 4.2.5 启动 `relayd`

同样，当前包内只提供二进制和环境文件示例。最小启动示例：

```bash
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

示例环境文件字段：

```dotenv
MESHLINK_RELAYD_LISTEN=:3478
MESHLINK_RELAYD_MANAGEMENT_ADDR=127.0.0.1:33073
MESHLINK_RELAYD_ADVERTISE_HOST=<management-public-ip-or-dns>
MESHLINK_RELAYD_BOOTSTRAP_TOKEN=replace-me
MESHLINK_RELAYD_SESSION_TTL=30s
```

`MESHLINK_RELAYD_ADVERTISE_HOST` 必须是客户端真实可达的地址，不能保留默认示例值。

### 4.3 Linux 客户端部署

#### 4.3.1 前提

1. 目标节点具备内核 WireGuard 支持
2. 已安装 `iproute2`
3. 以 root 或具备等效权限运行 `meshlinkd`

#### 4.3.2 安装与配置

```bash
sudo dpkg -i dist/deb/meshlink-client_*_amd64.deb
sudoedit /etc/meshlink/client.toml
```

最小配置示例：

```toml
management_addr = "192.0.2.10:33073"
```

只填写 `management_addr` 时，客户端会自动：

1. 使用默认 bootstrap token `meshlink-dev-token`
2. 按同一 host 推导 `signal_addr=:10000`、`relay_addr=:3478`、`stun_addr=:3479`
3. 自动生成并持久化 `private_key` / `public_key`
4. Linux 默认接口名为 `sdwan0`
5. 默认监听端口为 `51820`

首次启动后会在配置文件同目录生成状态文件：

```text
/etc/meshlink/client.state.json
```

如果需要覆盖默认行为，可额外配置：

```toml
node_name = "client-a"
signal_addr = "192.0.2.10:10000"
relay_addr = "192.0.2.10:3478"
stun_addr = "192.0.2.10:3479"
bootstrap_token = "replace-me"
interface_name = "sdwan0"
listen_port = 51820
advertise_host = "198.51.100.20"
```

#### 4.3.3 启动客户端

仓库已提供 systemd unit：

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now meshlink-client
sudo systemctl status meshlink-client --no-pager
```

手工运行方式：

```bash
/usr/bin/meshlinkd --config /etc/meshlink/client.toml
```

### 4.4 Windows 客户端部署

#### 4.4.1 生成或取得 zip 包

```bash
make package-windows
```

输出示例：

```text
dist/windows/x86_64-pc-windows-gnu/meshlink-client_1.0.0_x86_64-pc-windows-gnu.zip
```

#### 4.4.2 部署步骤

1. 解压 zip 到本地目录
2. 保持 `meshlinkd.exe`、`tunnel.dll`、`wireguard.dll`、`wintun.dll` 位于同一目录
3. 将 `client.example.toml` 复制为 `client.toml`
4. 用管理员 PowerShell 执行 `run-meshlinkd.ps1`

示例配置：

```toml
management_addr = "192.0.2.10:33073"
```

只填写 `management_addr` 时，Windows 客户端也会自动生成 `client.state.json`，并自动补齐默认 `signal` / `relay` / `stun` 地址与 WireGuard 密钥。

启动命令：

```powershell
powershell.exe -ExecutionPolicy Bypass -File .\run-meshlinkd.ps1
```

#### 4.4.3 Windows 注意事项

1. 必须使用管理员 PowerShell
2. Windows 路径依赖包内 runtime DLL，不能单独移动 `meshlinkd.exe`
3. 当前推荐验证路径见 `tests/windows-vm/README.md`
4. `client.state.json` 与 `client.toml` 必须保留在同一目录，重启后会复用同一设备身份

### 4.5 Docker Compose 开发部署

仓库提供了开发态 compose 示例：`deploy/docker-compose/docker-compose.yml`。

启动前提：

1. 已在 `server/` 可成功构建镜像
2. 仅用于开发或演示，不应替代正式运维编排

启动示例：

```bash
docker compose -f deploy/docker-compose/docker-compose.yml up --build
```

## 5. 管理手册

### 5.1 管理台登录

管理台入口：

```text
http://<management-host>:8080/admin/
```

登录使用 `MESHLINK_MANAGEMENTD_ADMIN_TOKEN`。登录后由 `managementd` 下发 HTTP-only session cookie。

当前限制：

1. 管理台使用静态 admin token
2. session 存在内存中，`managementd` 重启后浏览器需要重新登录

### 5.2 管理台能力

当前 `/admin/` 支持：

1. Overview
   查看三服务健康状态、设备总数、在线/离线数量、最近审计事件。
2. Devices
   查看设备列表、筛选设备、查看 peer 视图。
3. Device Detail
   修改展示名、标签、`advertised_routes`，执行启用、禁用、删除。
4. Services
   查看 `managementd`、`signald`、`relayd` 的运行态与只读配置摘要。
5. Audit
   查看持久化的管理动作和关键系统事件。

### 5.3 常用运维命令

#### `managementd`

```bash
sudo systemctl restart meshlink-managementd
sudo journalctl -u meshlink-managementd -n 100 --no-pager
curl -fsS http://127.0.0.1:8080/admin/ >/dev/null
```

#### `signald`

```bash
pgrep -af signald
curl -fsS http://127.0.0.1:18080/internal/status/v1/service
tail -n 100 /var/log/meshlink-signald.log
```

#### `relayd`

```bash
pgrep -af relayd
curl -fsS http://127.0.0.1:18081/internal/status/v1/service
tail -n 100 /var/log/meshlink-relayd.log
```

#### Linux 客户端

```bash
sudo systemctl restart meshlink-client
sudo journalctl -u meshlink-client -n 100 --no-pager
sudo ip link show sdwan0
sudo wg show
```

### 5.4 SQLite 状态管理

`managementd` 当前以 SQLite 作为唯一管理面状态源，默认文件：

```text
/var/lib/meshlink/management.db
```

建议：

1. 把该文件纳入备份
2. 做变更窗口前先备份状态库
3. 删除设备前先确认该节点确实已退役，因为审计和设备元数据都依赖该库

## 6. 使用手册

### 6.1 客户端配置字段说明

最常用字段如下：

1. `node_name`
   节点展示名。
2. `management_addr`
   `managementd` gRPC 地址。
3. `signal_addr`
   `signald` 地址；启用 NAT 打洞时使用。
4. `relay_addr`
   `relayd` 控制地址；直连失败时申请 relay session。
5. `stun_addr`
   STUN 地址；未显式配置时，客户端会按 `signal_addr` 主机名推导 `:3479`。
6. `bootstrap_token`
   引导令牌；默认值是 `meshlink-dev-token`。
7. `public_key`
   设备 WireGuard 公钥。
8. `private_key`
   设备 WireGuard 私钥；Linux 直连/NAT 路径和 Windows 路径都需要。
9. `interface_name`
   本地接口名，Linux 常见为 `sdwan0`，Windows 可用 `MeshLink`。
10. `listen_port`
    本地 WireGuard 监听端口。
11. `advertise_host`
    Linux 静态直连场景下向控制面发布的可达地址。
12. `advertised_routes`
    节点后方静态网段。
13. `client.state.json`
    客户端首次启动后自动生成的本地状态文件，保存 `private_key` / `public_key`。

### 6.2 典型使用模式

#### 模式 A：最小控制面验证

适用于 Phase 01/02 风格验证，只需要：

```toml
management_addr = "192.0.2.10:33073"
```

该模式现在会自动生成身份并自动推导同 host 的 `signal` / `relay` / `stun` 地址，适合作为默认安装方式。

#### 模式 B：Linux 静态直连

需要补充：

```toml
private_key = "replace-me"
listen_port = 51820
advertise_host = "198.51.100.20"
```

适用于公网或已知可达地址的 Linux 节点。

#### 模式 C：NAT 打洞 + Relay fallback

需要补充：

```toml
management_addr = "192.0.2.10:33073"
signal_addr = "192.0.2.10:10000"
relay_addr = "192.0.2.10:3478"
stun_addr = "192.0.2.10:3479"
listen_port = 51820
```

适用于多 NAT 环境。正常情况下优先直连；只有打洞失败或 direct path 丢失时才进入 relay。

### 6.3 路由发布

客户端可声明：

```toml
advertised_routes = ["10.20.0.0/24"]
```

控制面的行为：

1. 在注册时校验 CIDR
2. 将其合并到其他 peer 看到的 `AllowedIPs`
3. 当客户端把 `advertised_routes` 清空后，服务端撤销此前发布的静态路由

当前约束：

1. 仅支持静态 IPv4 CIDR
2. 不允许 `0.0.0.0/0`
3. 不允许与 overlay CIDR 重叠
4. 不允许与其他设备已发布路由重叠

### 6.4 基本验收

#### Linux

```bash
sudo systemctl status meshlink-client --no-pager
sudo journalctl -u meshlink-client -n 50 --no-pager
sudo ip addr show sdwan0
sudo wg show
ping <peer-overlay-ip>
```

#### Windows

1. 确认 `run-meshlinkd.ps1` 成功执行
2. 检查 `C:\ProgramData\MeshLink\<interface>.conf`
3. 验证 Windows 到对端 overlay 地址可达

#### 管理台

1. 打开 `/admin/`
2. 确认设备显示为在线
3. 确认 Services 页面能看到 `signald` / `relayd` 内部状态

## 7. 常见问题

### 7.1 客户端无法注册

优先检查：

1. `management_addr` 是否可达
2. `bootstrap_token` 是否与 `managementd` 一致
3. 管理节点 `33073/tcp` 是否放通

### 7.2 管理台可以打开，但看不到 `signald` 或 `relayd` 状态

优先检查：

1. `managementd` 环境中的 `MESHLINK_SIGNALD_STATUS_URL` / `MESHLINK_RELAYD_STATUS_URL`
2. `signald` `18080` 与 `relayd` `18081` 是否在本机可访问
3. 对应进程是否实际启动

### 7.3 NAT 场景下无法直连

优先检查：

1. `signal_addr`、`relay_addr`、`stun_addr` 是否配置正确
2. `listen_port` 是否稳定且可用
3. 如果你走的是 Linux 静态直连而不是 NAT 打洞，`advertise_host` 是否填成真实可达地址
4. `relayd` 的 `advertise_host` 是否对客户端真实可达

### 7.4 服务重启后管理台需要重新登录

这是当前预期行为。管理员 session 保存在 `managementd` 内存里，不持久化到 SQLite。

## 8. 仓库内参考入口

1. `README.md`
2. `docs/architecture/system-overview.md`
3. `docs/architecture/control-plane.md`
4. `docs/architecture/admin-console.md`
5. `docs/api/service-contracts.md`
6. `tests/nat-lab/README.md`
7. `tests/windows-vm/README.md`
