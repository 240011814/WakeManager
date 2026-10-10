# WakeManager (Rust 版)

基于官方 [`esp-idf-template`](https://github.com/esp-rs/esp-idf-template)（std 路线）
+ `esp-idf-svc` 0.53 的 WakeManager 重写，目标芯片 ESP32-S3，ESP-IDF v6.1。

当前阶段：**P1+P2 完整移植** — 全部业务模块已从 C 版移植（配置/地址簿/WOL/HTTP/MQTT/
LED/看门狗/SNTP/温度），BLE 配网沿用 P3 已验证的 FFI 方案。

## 模块对照（C → Rust）

| C 组件 | Rust 模块 | 实现方式 |
| --- | --- | --- |
| `app_config.h` | `src/app_config.rs` | 常量 |
| `config_store.c` | `src/config_store.rs` | NVS blob（260 字节，与 C 版逐字节兼容） |
| `mac_store.c` | `src/mac_store.rs` | NVS blob（341 字节，与 C 版逐字节兼容） |
| `device_info.c` | `src/device_info.rs` | FFI |
| `wol.c` | `src/wol.rs` | `std::net::UdpSocket` + netif FFI |
| `chip_temp.c` | `src/chip_temp.rs` | temperature_sensor FFI |
| `time_sync.c` | `src/time_sync.rs` | esp_sntp FFI（与 C 调用序列一致） |
| `watchdog.c` | `src/watchdog.rs` | esp_task_wdt FFI |
| `led_status.c` | `src/led_status.rs` | GPIO FFI + 独立线程 |
| `wifi_prov.c` | `src/wifi_prov.rs` | network_provisioning FFI（P3 已验证） |
| `control_http.c` | `src/control_http.rs` | `EspHttpServer`（index.html include_str 内嵌） |
| `control_mqtt.c` | `src/control_mqtt.rs` | `EspMqttClient`（回调）+ FFI publish/subscribe |
| `app_main.c` | `src/main.rs` | 编排入口 |
| （status 快照辅助） | `src/netinfo.rs` | netif/wifi FFI |

JSON 构造统一用 `serde_json`（替代 C 的手写 sb_add / cJSON）。

## 项目生成方式

本项目由官方模板生成（`cargo-generate`），随后合并了 FFI 原型代码：

```bash
cargo install cargo-generate ldproxy
cargo generate --path esp-idf-template/cargo --name wol-rs \
    --define mcu=esp32s3 --define advanced=true --define espidfver=v6.1 \
    --define git=false --define installdir=global \
    --define devcontainer=false --define wokwi=false --define ci=false
```

模板关键产物：
- `rust-toolchain.toml` → `channel = "esp"`（espup 安装的 Xtensa 工具链）
- `.cargo/config.toml` → target、`ldproxy` linker、`ESP_IDF_VERSION=v6.1`、
  `ESP_IDF_TOOLS_INSTALL_DIR=global`（复用 `~/.espressif` 已有工具链）
- `build.rs` → `embuild::espidf::sysenv::output()`
- `sdkconfig.defaults` → 模板的栈大小设置 + 本项目 BLE 配网配置

## 环境准备（一次性）

```bash
# 1. rustup（如已安装跳过）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

# 2. espup：安装 Xtensa Rust 工具链（ESP32-S3 是 Xtensa 架构）
cargo install espup --locked
espup install --targets esp32s3 --std

# 3. 每个新 shell 需要激活
. ~/export-esp.sh          # LIBCLANG_PATH（bindgen 用）
. "$HOME/.cargo/env"       # cargo/rustup
```

## 构建与烧录

```bash
cd wol-rs
cargo build                                    # rust-toolchain.toml 自动选 esp 工具链；
                                               # 首次构建会完整编译 ESP-IDF（10~20 分钟）
cargo espflash flash --monitor \
    --partition-table ../partitions.csv        # 自定义分区表需显式传入（esp-idf-sys 已知限制）
```

> `sdkconfig.defaults` 里**不能**定义 `CONFIG_PARTITION_TABLE_CUSTOM*`
> （esp-idf-sys 已知限制），分区表在烧录时通过 `--partition-table` 指定。

## FFI 绑定 network_provisioning 的机制

`Cargo.toml`：

```toml
[[package.metadata.esp-idf-sys.extra_components]]
remote_component = { name = "espressif/network_provisioning", version = "1.*" }
bindings_header = "bindings.h"   # #include network_provisioning/{manager,scheme_ble}.h
# 不设 bindings_module → 绑定并入 esp_idf_sys 根，与主 IDF 绑定共享同一套类型
```

由此在 `esp_idf_sys` 中直接可用：

- `network_prov_mgr_init/deinit/is_wifi_provisioned/start_provisioning/...`
- `network_prov_scheme_ble`、`network_prov_scheme_ble_event_cb_free_btdm`
- 事件基 `NETWORK_PROV_EVENT`、`PROTOCOMM_TRANSPORT_BLE_EVENT`
- 枚举常量 `network_prov_security_NETWORK_PROV_SECURITY_1` 等

**踩坑记录（C → Rust 绑定差异）：**

| C | Rust 绑定实际名 |
| --- | --- |
| `WIFI_INIT_CONFIG_DEFAULT()` 宏 | 不可见 → 按 sdkconfig 值手工构造 `wifi_init_config_t`（见 main.rs） |
| `NETWORK_PROV_EVENT_HANDLER_NONE` / `..._FREE_BTDM` 复合字面量宏 | 不可见 → 手工构造 `network_prov_event_handler_t { event_cb, user_data }` |
| `network_prov_security_t_NETWORK_PROV_SECURITY_1` | `network_prov_security_NETWORK_PROV_SECURITY_1`（`_t` 是 type alias，常量名无 `_t`） |
| `ssize_t`（protocomm 回调签名） | 绑定里是 `isize` |
| `esp_idf_sys::*` glob import | 内含 `log` struct → `log::info!` 有歧义，需写 `::log::info!` |

## 验证清单（P1+P2 完整移植，2026-10-10 真机通过）

- [x] `cargo build` 编译通过（0 错误；16 个 dead-code/命名风格警告）
- [x] 烧录后启动：chip info → LED → NVS → 配置/地址簿/看门狗/温度 →
      `Device is already provisioned, starting Wi-Fi STA` → IP 172.16.9.207
- [x] **NVS blob 兼容**：C 固件的地址簿（1 条 `64:4E:D7:67:FB:0E`）与默认配置被 Rust 固件直接读取
- [x] **SNTP**：`Clock synchronised: 2026-10-10 17:20:24`（tz=CST-8 本地时间）
- [x] **HTTP**：`/`（index.html）、`/api/status`、`/api/config` GET/POST（保存+MQTT 热重启）、
      `/api/wake`（发包 + 缺 mac 400）、`/api/mac` 全部通过；非法端口 400
- [x] **MQTT**（broker mqtt://demonkingqianjing.cloud:21883）：
      广播发现/状态应答、wake（`ok: magic packet sent`）、
      mac set/del/clear 全分支（含 `error: address book full`/`mac not found`/`invalid MAC`/
      `unsupported command` 等 C 版文案）、mac_get 列表、无效 JSON 报错
- [x] **看门狗**：30s TWDT + panic，长时间运行无重启
- [ ] BLE 重新配网（需手机 ESP Provisioning App：先擦除 NVS 命名空间 `wifi.prov`）

> 注：`partitions.csv` 的 factory 分区从 0x180000 扩到 0x300000（Rust 镜像 1.8MB 超过
> 1.5MB；C 固件共享该表，扩分区对 C 无影响）。

## 目录

```
wol-rs/
├── Cargo.toml              # 模板骨架 + esp-idf-sys extra_components（network_provisioning/mqtt）+ serde
├── build.rs                # 模板自带：embuild::espidf::sysenv::output()
├── rust-toolchain.toml     # 模板自带：channel = "esp"
├── bindings.h              # bindgen 入口头文件（network_provisioning + chip_info/flash）
├── .cargo/config.toml      # 模板自带：target/ldproxy/ESP_IDF_VERSION=v6.1/global
├── sdkconfig.defaults      # 模板栈大小 + NimBLE + protocomm security 1
└── src/
    ├── main.rs             # 编排入口（对应 app_main.c）
    ├── app_config.rs       # 编译期常量（app_config.h）
    ├── config_store.rs     # NVS 配置（blob 260B 与 C 逐字节兼容）
    ├── mac_store.rs        # NVS 地址簿（blob 341B 与 C 逐字节兼容）
    ├── device_info.rs      # MAC / 设备 id
    ├── netinfo.rs          # IP/掩码/SSID/RSSI/空闲堆
    ├── wol.rs              # magic packet（std UdpSocket）+ 子网广播推导
    ├── chip_temp.rs        # 片内温度（temperature_sensor FFI）
    ├── time_sync.rs        # SNTP（esp_sntp FFI）
    ├── watchdog.rs         # TWDT（esp_task_wdt FFI）
    ├── led_status.rs       # GPIO48 + 独立线程（FFI + std thread）
    ├── wifi_prov.rs        # BLE 配网（network_provisioning FFI，P3 已验证）
    ├── control_http.rs     # EspHttpServer（index.html include_str 内嵌）
    └── control_mqtt.rs     # EspMqttClient 回调 + FFI publish/subscribe
```
