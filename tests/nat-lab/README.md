# MeshLink VM Lab

这个目录提供 MeshLink 的 libvirt/QEMU 实验室脚本，当前支持两种拓扑模式：

1. `flat`
2. `dual-nat`

`flat` 保持 Phase 01-04 和轻量 Phase 05 回归的三节点最小实验室，`dual-nat` 是 Phase 05/06 的主验证环境，用来覆盖真实的多 NAT 打洞、relay fallback 和 direct-path recovery。

## 前提

1. 宿主机已安装 `virsh`、`virt-install`、`qemu-img`、`ssh`、`scp`
2. libvirt `default` network 与目标 storage pool 处于 active
3. 已准备 Linux cloud image
4. 本机存在可注入到 VM 的 SSH 公钥，或在环境文件里显式设置
5. 新建 VM 时会通过 cloud-init 安装 WireGuard、ping、router 所需工具，并为实验用户授予免密 sudo

## 配置

1. 复制 `tests/nat-lab/libvirt.env.example` 为 `tests/nat-lab/libvirt.env`
2. 至少填写 `MESHLINK_BASE_IMAGE`
3. 按阶段选择拓扑：
   `MESHLINK_LAB_TOPOLOGY=flat` 用于 Phase 01-04 和轻量 Phase 05 回归
   `MESHLINK_LAB_TOPOLOGY=dual-nat` 用于 Phase 05/06 主验收
4. 如需自定义网络、pool、SSH 用户或静态 IP，也在该文件里修改

切换拓扑后建议先执行 `./tests/nat-lab/destroy-lab.sh` 再重建，确保静态网络与 cloud-init 配置一致。

## 拓扑

### `flat`

默认三节点：

1. `mgmt-1`
2. `client-a`
3. `client-b`

三台 VM 都连接到 libvirt `default` 网络，适合 Phase 01-05 的现有自动化脚本。

### `dual-nat`

五节点：

1. `mgmt-1`
2. `nat-a`
3. `nat-b`
4. `client-a`
5. `client-b`

网络结构：

1. `mgmt-1` 只连接上游网络
2. `nat-a` 连接 `upstream-net` + `nat-a-lan`
3. `nat-b` 连接 `upstream-net` + `nat-b-lan`
4. `client-a` 只连接 `nat-a-lan`
5. `client-b` 只连接 `nat-b-lan`

`nat-a` 与 `nat-b` 通过 cloud-init 自动开启 `ip_forward` 和 `MASQUERADE`，并可通过脚本注入“阻断 peer-to-peer WireGuard UDP、保留 management/signal/relay 可达”的故障场景。
`run-phase05.sh` 与 `run-phase06.sh` 在这个拓扑下都会先做 fail-fast 预检；如果宿主机对某台 VM 的静态 IP 不可达，脚本会直接打印诊断并要求重建实验室，而不是卡在长时间 SSH 重试里。

## 命令

创建拓扑：

```bash
./tests/nat-lab/create-lab.sh
```

运行 Phase 01/02 验收：

```bash
./tests/nat-lab/run-phase01-02.sh
```

运行 Phase 03 二进制直拷验收：

```bash
./tests/nat-lab/run-phase03.sh
```

运行 Phase 03 deb 安装包验收：

```bash
./tests/nat-lab/run-phase03-deb.sh
```

运行 Phase 05 信令与动态 endpoint 验收：

```bash
MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase05.sh
```

运行 Phase 06 relay fallback / recovery 验收：

```bash
MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase06.sh
```

销毁拓扑：

```bash
./tests/nat-lab/destroy-lab.sh
```

## 验收内容

`run-phase01-02.sh` 会在宿主机编译 `managementd` 与 `meshlinkd`，拷贝到 VM，并验证：

1. `mgmt-1` 成功启动管理服务
2. `client-a`、`client-b` 均完成注册
3. 两个客户端都能收到 `tracked_peers=1` 的配置事件
4. 两个客户端都记录 `peer_added=1`

`run-phase03.sh` 会额外：

1. 在宿主机生成 `client-a` 和 `client-b` 的临时 WireGuard 密钥对
2. 为两个客户端写入 `private_key`、`listen_port` 和 `advertise_host`
3. 以 root 权限启动 `meshlinkd`，检查 `wg show`
4. 验证 `client-a` 与 `client-b` 之间的 overlay ping
5. 该脚本当前只支持 `flat` 拓扑

`run-phase03-deb.sh` 会额外：

1. 使用宿主机 `make package-deb` 生成的 `dist/deb/*.deb`
2. 在 `mgmt-1` 安装 `meshlink-managementd`、`meshlink-signald`、`meshlink-relayd`
3. 在 `client-a` 和 `client-b` 安装 `meshlink-client`
4. 覆盖 `/etc/default/meshlink-managementd.env` 与 `/etc/meshlink/client.toml`
5. 通过 `systemctl restart` 启动 `meshlink-managementd` 和 `meshlink-client`
6. 验证 journal、`wg show` 和 overlay ping
7. 该脚本当前只支持 `flat` 拓扑

`run-phase05.sh` 会额外：

1. 在 `mgmt-1` 同时启动 `managementd` 和 `signald`
2. 客户端启用 `signal_addr`，并通过 `signald` 的 STUN 与 candidate exchange 路径建立 peer endpoint override
3. 在 `dual-nat` 下验证 `wg show` 最终收敛到对端 NAT WAN IP + WireGuard 监听端口
4. 验证 overlay ping 与客户端打洞成功日志
5. `flat` 仍可运行，但 `dual-nat` 是主验收拓扑

`run-phase06.sh` 当前提供完整的 relay fallback / recovery 验收：

1. 验证 `mgmt-1`、`nat-a`、`nat-b`、`client-a`、`client-b` 都可达
2. 在 `mgmt-1` 启动 `managementd`、`signald`、`relayd`
3. 验证客户端先建立 direct NAT-WAN path
4. 注入 peer-to-peer WireGuard UDP drop 规则并确认客户端切到 `mgmt-1:<dynamic-relay-port>`
5. 在 relay active 期间继续验证 overlay ping
6. 清理 drop 规则并确认客户端恢复回 direct NAT-WAN path
7. 采集 `iptables-save`、`conntrack`、`wg show` 与服务日志，作为故障与恢复断言的留档
