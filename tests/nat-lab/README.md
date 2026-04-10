# MeshLink VM Lab

这个目录提供 Phase 01/02 的最小 libvirt 实验室骨架，默认使用三台 Linux VM：

1. `mgmt-1`
2. `client-a`
3. `client-b`

## 前提

1. 宿主机已安装 `virsh`、`virt-install`、`qemu-img`、`ssh`、`scp`
2. libvirt `default` network 与目标 storage pool 处于 active
3. 已准备 Linux cloud image
4. 本机存在可注入到 VM 的 SSH 公钥，或在环境文件里显式设置

## 配置

1. 复制 `tests/nat-lab/libvirt.env.example` 为 `tests/nat-lab/libvirt.env`
2. 至少填写 `MESHLINK_BASE_IMAGE`
3. 如需自定义网络、pool、SSH 用户或静态 IP，也在该文件里修改

默认实现：

1. 绑定 libvirt `default` network
2. 绑定 libvirt `default` storage pool
3. 使用 `virt-install --cloud-init`
4. 在 guest 内只运行 Phase 01/02 所需的 `managementd` 与 `meshlinkd`

## 命令

创建拓扑：

```bash
./tests/nat-lab/create-lab.sh
```

运行 Phase 01/02 验收：

```bash
./tests/nat-lab/run-phase01-02.sh
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

当前骨架只覆盖 Phase 01/02。Phase 03 的 WireGuard 接口与 overlay 连通性验证，等 `TASK-006` 完成后补入这套实验室。
