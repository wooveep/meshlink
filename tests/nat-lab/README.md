# MeshLink VM Lab

这个目录提供 Phase 01/03 的最小 libvirt 实验室骨架，默认使用三台 Linux VM：

1. `mgmt-1`
2. `client-a`
3. `client-b`

## 前提

1. 宿主机已安装 `virsh`、`virt-install`、`qemu-img`、`ssh`、`scp`
2. libvirt `default` network 与目标 storage pool 处于 active
3. 已准备 Linux cloud image
4. 本机存在可注入到 VM 的 SSH 公钥，或在环境文件里显式设置
5. 新建 VM 时会通过 cloud-init 安装 `wireguard-tools`、`iputils-ping` 并为实验用户授予免密 sudo

## 配置

1. 复制 `tests/nat-lab/libvirt.env.example` 为 `tests/nat-lab/libvirt.env`
2. 至少填写 `MESHLINK_BASE_IMAGE`
3. 如需自定义网络、pool、SSH 用户或静态 IP，也在该文件里修改

默认实现：

1. 绑定 libvirt `default` network
2. 绑定 libvirt `default` storage pool
3. 使用 `virt-install --cloud-init`
4. 在 guest 内运行 Phase 01/03 所需的 `managementd` 与 `meshlinkd`

如果这些 VM 是在本次变更之前创建的，建议先执行 `./tests/nat-lab/destroy-lab.sh` 再重建，以确保 guest 里装好 WireGuard 和 ping 工具。

## 命令

创建拓扑：

```bash
./tests/nat-lab/create-lab.sh
```

运行 Phase 01/02 验收：

```bash
./tests/nat-lab/run-phase01-02.sh
```

运行 Phase 03 验收：

```bash
./tests/nat-lab/run-phase03.sh
```

销毁拓扑：

```bash
./tests/nat-lab/destroy-lab.sh
```

## 验收内容

`run-phase01-02.sh` 会在宿主机编译 `managementd` 与 `meshlinkd`，拷贝到三台 VM，并验证：

1. `mgmt-1` 成功启动管理服务
2. `client-a`、`client-b` 均完成注册
3. 两个客户端都能收到 `tracked_peers=1` 的配置事件
4. 两个客户端都记录 `peer_added=1`

`run-phase03.sh` 会额外：

1. 在宿主机生成 `client-a` 和 `client-b` 的临时 WireGuard 密钥对
2. 为两个客户端写入 `private_key`、`listen_port` 和 `advertise_host`
3. 以 root 权限启动 `meshlinkd`，检查 `wg show`
4. 验证 `client-a` 与 `client-b` 之间的 overlay ping
