# VM Topology

## Default topology

1. `mgmt-1`
   Runs `managementd`
2. `client-a`
   Runs `meshlinkd`
3. `client-b`
   Runs `meshlinkd`

## Future extensions

1. `nat-a`
2. `nat-b`
3. `relay-1`

## Platform

Use QEMU/libvirt as the default orchestration target.
