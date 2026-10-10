# WakeManager

基于 **ESP32-S3** 的网络唤醒（Wake-on-LAN）控制器：BLE 配网接入 Wi-Fi 后，通过
HTTP（单页 Web UI + REST API）与 MQTT 远程唤醒主机、管理 MAC/IP 地址簿、上报设备状态。

项目已从 C（ESP-IDF / FreeRTOS）**完整改写为 Rust**（`esp-idf-svc`，std 路线），
ESP-IDF v6.1，目标芯片 ESP32-S3。原 C 实现已移除。

## 目录结构

```
WakeManager/
├── partitions.csv        # 分区表（factory 0x300000，烧录时显式传入）
├── CLAUDE.md             # 项目说明 / 开发注意事项
└── wol-rs/               # Rust 固件工程（cargo 项目）
    ├── Cargo.toml
    ├── build.rs
    ├── bindings.h        # bindgen 入口（network_provisioning 等）
    ├── sdkconfig.defaults
    └── src/              # 全部业务模块（config/mac/wol/http/mqtt/led/wifi…）
```

## 构建与烧录

详见 [`wol-rs/README.md`](wol-rs/README.md)。简述：

```bash
# 一次性环境准备（见 wol-rs/README.md）
# 每个新 shell 需要激活：
. ~/export-esp.sh
. "$HOME/.cargo/env"

cd wol-rs
cargo build
cargo espflash flash --monitor --partition-table ../partitions.csv
```

## HTTP API

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/` | 内嵌单页 Web UI |
| GET | `/api/status` | 状态 JSON（`id`/`ip`/`ssid`/`rssi`/`heap`/`broadcast`/`mac_count`/`temp_c`/`time`…） |
| GET / POST | `/api/config` | 读取 / 保存运行配置（NVS） |
| GET | `/api/wake` | 发送魔术包，`?mac=<MAC>` 必传，`?ip=<IP>` 可选 |
| GET | `/api/mac` | MAC/IP 地址簿 |

## MQTT 协议

设备 ID = Wi-Fi STA MAC 的 12 位大写十六进制。订阅主题（固定为编译期常量 + `/<id>`）：

- `wakemanager/broadcast`（共享）：设备发现，`{"cmd":"status"}` → 状态应答
- `wakemanager/cmd/<id>`：`{"cmd":"wake","mac":"AA:BB:CC:DD:EE:FF","ip":"..."}`（`mac` 必传）
- `wakemanager/mac/<id>`：地址簿 `set/add/update`、`del/delete/remove`、`clear`
- `wakemanager/mac/get/<id>`：查询地址簿

发布主题：`wakemanager/status`。

> 详细说明、命令示例与开发注意事项见 [`CLAUDE.md`](CLAUDE.md)。