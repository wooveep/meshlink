# MeshLink Windows VM Validation

这个目录提供一个“Linux libvirt 实验室 + 额外 Windows VM”的验证路径，用来覆盖当前 Embedded Runtime 阶段的真实 Windows 联调，而不引入第二套协议或第二套路由模型。它同时支持：

1. 纯手工安装与验证
2. 安装 `qemu-ga` 后，通过 libvirt guest agent 从宿主机执行命令和读取结果

## 目标

推荐把 Windows VM 挂到 `dual-nat` 的 `nat-a-lan` 或 `nat-b-lan`，继续复用现有的：

1. `mgmt-1`
2. `nat-a`
3. `nat-b`
4. 一个 Linux 客户端

这样可以在不改动服务端契约的前提下验证：

1. Windows 注册与 `SyncConfig`
2. Windows 对 Linux 的直连打洞
3. relay fallback 与 direct-path recovery
4. Linux 发布静态路由后，Windows 对 `AllowedIPs` 的吸收与撤销

## 前提

1. 已经准备好 Windows 安装 ISO，或者宿主机上已有可复用的 Windows qcow2 磁盘
2. libvirt 宿主机已能运行 `tests/nat-lab/` 的 `dual-nat` 拓扑
3. Windows 来宾里会手工设置静态 IP
4. `make package-windows` 所需的 `tunnel.dll`、`wireguard.dll` 与
   `wintun.dll` 已经按固定版本目录 stage 好
5. 如果要走 qemu guest agent 路径，准备好 `virtio-win.iso` 或其他能在来宾内安装 `qemu-ga` 的方式

## 1. 先起 Linux dual-NAT 实验室

```bash
MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/create-lab.sh
MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase05.sh
```

如果你要一起验证 relay fallback，建议 `mgmt-1`、`nat-a`、`nat-b` 和 Linux peer 都保持运行。

## 2. 生成 Windows 包

优先走仓库内的打包脚本：

```bash
make package-windows
```

如果 runtime DLL 还没 stage，好处是现在 Linux 宿主机会自动先补：

```bash
make windows-runtime
make package-windows
```

如果当前 Linux 宿主机没有 Windows 交叉编译工具链，也可以在 Windows 主机或 CI 先构建 `meshlinkd.exe`，再把现成二进制交给打包脚本：

```bash
MESHLINK_WINDOWS_BINARY=/absolute/path/to/meshlinkd.exe make package-windows
```

输出位置：

```text
dist/windows/<target>/meshlink-client_<version>_<target>.zip
```

## 3. 创建 Windows VM

1. 复制 `tests/windows-vm/windows.env.example` 为 `tests/windows-vm/windows.env`
2. 至少填写：
   `MESHLINK_WINDOWS_PACKAGE`
3. 选择一种供给方式：
   全新安装：填写 `MESHLINK_WINDOWS_ISO`
   直接导入：填写 `MESHLINK_WINDOWS_BASE_DISK`
4. 选择网络：
   `MESHLINK_WINDOWS_ATTACH_NETWORK=nat-a-lan` 表示把 Windows 放到 NAT A 后面
   `MESHLINK_WINDOWS_ATTACH_NETWORK=nat-b-lan` 表示把 Windows 放到 NAT B 后面
5. 如果走磁盘导入：
   `MESHLINK_WINDOWS_CLONE_MODE=backing` 创建轻量 overlay，适合临时验证
   `MESHLINK_WINDOWS_CLONE_MODE=copy` 创建完整副本，更独立但更慢
6. 可选：
   `MESHLINK_WINDOWS_VIRTIO_ISO=/path/to/virtio-win.iso`
   用于在来宾里安装 `qemu-ga`

创建 VM：

```bash
./tests/windows-vm/check-package.sh
./tests/windows-vm/create-vm.sh
```

脚本会打印推荐的静态 IPv4、网关和 DNS。首次安装完成后，或者首次从模板导入后，在 Windows 里按这些参数手工设置网卡。

默认情况下，`create-vm.sh` 还会给域 XML 加上 `org.qemu.guest_agent.0`
channel。对已经存在的 VM，可以执行：

```bash
./tests/windows-vm/enable-qga.sh
```

如果 `windows.env` 里配置了 `MESHLINK_WINDOWS_VIRTIO_ISO`，这个脚本也会顺带把
virtio ISO 挂到现有 VM 上。

如果 VM 已经在跑，而且你只是重新打了新的 Windows zip，可以执行：

```bash
./tests/windows-vm/refresh-package-iso.sh
```

这会把当前 `MESHLINK_WINDOWS_PACKAGE` 重新封成 VM 已挂载的
`meshlink-package.iso` 并热更新 CD-ROM 介质。

## 4. 为 Windows 节点准备 NAT 端口映射

如果 Windows VM 挂在 `nat-a-lan` 且 WireGuard 监听端口是 `51820`，执行：

```bash
MESHLINK_LAB_TOPOLOGY=dual-nat \
MESHLINK_WINDOWS_SIDE=nat-a \
MESHLINK_WINDOWS_IP=10.10.1.20 \
MESHLINK_WINDOWS_WG_PORT=51820 \
./tests/windows-vm/prepare-dual-nat.sh map
```

如果挂在 `nat-b-lan`，把 `MESHLINK_WINDOWS_SIDE` 和 `MESHLINK_WINDOWS_IP` 改成对应值。

## 5. 在 Windows 里启动 MeshLink

1. 打开附带的只读光盘，把 `meshlink-client_*.zip` 解压到本地目录
2. 确认压缩包里同时包含 `meshlinkd.exe`、`tunnel.dll`、`wireguard.dll`
   和 `wintun.dll`
3. 把 `client.example.toml` 复制成 `client.toml`，然后修改 `client.toml`
4. 使用管理员 PowerShell 启动：

```powershell
powershell.exe -ExecutionPolicy Bypass -File .\run-meshlinkd.ps1
```

推荐配置示例：

```toml
node_name = "win-a"
management_addr = "replace-with-mgmt-ip:33073"
signal_addr = "replace-with-mgmt-ip:10000"
relay_addr = "replace-with-mgmt-ip:3478"
stun_addr = "replace-with-mgmt-ip:3479"
bootstrap_token = "meshlink-dev-token"
public_key = "replace-me"
private_key = "replace-me"
interface_name = "MeshLink"
listen_port = 51820
log_level = "info"
```

这里的 `replace-with-mgmt-ip` 应该替换成当前实验室的 `MESHLINK_MGMT_IP`，默认常见值是 `192.168.122.201`，但如果你的 libvirt 默认网段不是 `192.168.122.0/24`，以实际环境为准。

### 可选：安装 qemu guest agent

如果你想从宿主机直接驱动 Windows 来宾：

1. 在来宾内挂载的 `virtio-win.iso` 里安装 `qemu-ga`
2. 启动并设置 `QEMU Guest Agent` 服务为自动启动
3. 回到宿主机执行：

```bash
./tests/windows-vm/qga.sh wait 180
./tests/windows-vm/qga.sh info
./tests/windows-vm/qga.sh powershell "Get-Service qemu-ga"
```

常用命令：

```bash
./tests/windows-vm/qga.sh powershell "Get-NetIPAddress -AddressFamily IPv4"
./tests/windows-vm/qga.sh powershell "Get-Process meshlinkd -ErrorAction SilentlyContinue"
./tests/windows-vm/qga.sh read 'C:\\ProgramData\\MeshLink\\MeshLink.conf'
```

### 自动化验收

如果 Linux dual-NAT 实验室已经在跑，并且 Windows VM 已经装好了 `qemu-ga`，可以直接执行：

```bash
MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/windows-vm/run-phase08-validation.sh
```

这个脚本会：

1. 刷新当前 Windows 包到已挂载 ISO
2. 解压并部署 `meshlinkd.exe` 与 runtime DLL
3. 生成新的 Windows WireGuard 密钥并写入 `client.toml`
4. 在 dual-NAT 实验室里自动避开与同侧 Linux client 冲突的
   Windows WireGuard 监听端口
5. 校验同侧 Linux peer 的 direct endpoint
6. 注入对侧 NAT drop 规则并校验 relay fallback
7. 清理故障规则后校验 direct recovery
8. 校验 Linux 广播路由在 Windows 端的 `AllowedIPs`、路由表与可达性
9. 校验路由撤销后的清理行为
10. 把 Windows 配置、日志和 Linux `wg show` 结果收集到
   `${MESHLINK_WINDOWS_VALIDATION_STATE_DIR:-$MESHLINK_LAB_STATE_DIR/windows-validation}`

## 6. 验证项

### 直连

1. 确认 Windows 端成功注册并收到 peer
2. 确认 Linux peer 也看到了 Windows 节点
3. 确认 Windows 生成的 `C:\ProgramData\MeshLink\<interface>.conf` 里 endpoint 指向对端 NAT WAN 地址
4. 验证 overlay ping 正常

如果装了 qemu-ga，可以额外从宿主机确认：

```bash
./tests/windows-vm/qga.sh read 'C:\\ProgramData\\MeshLink\\MeshLink.conf'
```

### Relay fallback

安装故障规则：

```bash
MESHLINK_LAB_TOPOLOGY=dual-nat \
MESHLINK_WINDOWS_SIDE=nat-a \
MESHLINK_WINDOWS_IP=10.10.1.20 \
MESHLINK_WINDOWS_WG_PORT=51820 \
./tests/windows-vm/prepare-dual-nat.sh drop
```

清理故障规则：

```bash
MESHLINK_LAB_TOPOLOGY=dual-nat \
MESHLINK_WINDOWS_SIDE=nat-a \
MESHLINK_WINDOWS_IP=10.10.1.20 \
MESHLINK_WINDOWS_WG_PORT=51820 \
./tests/windows-vm/prepare-dual-nat.sh clear
```

验证点：

1. 注入 drop 后，overlay 流量还能继续通
2. Windows 端日志出现 relay fallback
3. 清理 drop 后，路径恢复为 direct

### 路由发布

推荐让 Linux peer 发布 `advertised_routes = ["10.20.0.0/24"]`，然后在 Windows 端确认：

1. `C:\ProgramData\MeshLink\<interface>.conf` 的 `AllowedIPs` 含 `10.20.0.0/24`
2. `Get-NetRoute -AddressFamily IPv4 | findstr 10.20.0.0` 能看到对应路由
3. Linux peer 撤销该路由后，上述记录消失

如果装了 qemu-ga，可以直接从宿主机执行：

```bash
./tests/windows-vm/qga.sh powershell "Get-NetRoute -AddressFamily IPv4 | Where-Object DestinationPrefix -eq '10.20.0.0/24'"
```
