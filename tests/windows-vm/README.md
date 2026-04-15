# MeshLink Windows VM Validation

这个目录提供一个“Linux libvirt 实验室 + 额外 Windows VM”的手工验证路径，用来覆盖当前 Embedded Runtime 阶段的真实 Windows 联调，而不引入第二套协议或第二套路由模型。

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
4. `make package-windows` 所需的 `tunnel.dll` 与 `wireguard.dll` 已经按固定版本目录 stage 好

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

创建 VM：

```bash
./tests/windows-vm/create-vm.sh
```

脚本会打印推荐的静态 IPv4、网关和 DNS。首次安装完成后，或者首次从模板导入后，在 Windows 里按这些参数手工设置网卡。

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
2. 确认压缩包里同时包含 `meshlinkd.exe`、`tunnel.dll` 和 `wireguard.dll`
3. 修改 `client.toml`
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

## 6. 验证项

### 直连

1. 确认 Windows 端成功注册并收到 peer
2. 确认 Linux peer 也看到了 Windows 节点
3. 确认 Windows 生成的 `C:\ProgramData\MeshLink\<interface>.conf` 里 endpoint 指向对端 NAT WAN 地址
4. 验证 overlay ping 正常

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
