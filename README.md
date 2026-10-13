# WakeManager (Rust 版)

基于官方 [`esp-idf-template`](https://github.com/esp-rs/esp-idf-template)（std 路线）
+ `esp-idf-svc` 0.53 的 WakeManager 重写，目标芯片 ESP32-S3，ESP-IDF v6.1。

当前阶段：**完整移植，C 版已移除** — 全部业务模块已从 C 版移植（配置/地址簿/WOL/HTTP/MQTT/
LED/看门狗/SNTP/温度），BLE 配网沿用已验证的 FFI 方案。内嵌 Web UI `index.html`
已随迁移移入 `src/index.html`（`control_http.rs` 用 `include_str!("index.html")` 内嵌）。

## 模块对照（C → Rust）

| C 组件 | Rust 模块 | 实现方式 |
| --- | --- | --- |
| `app_config.h` | `src/app_config.rs` | 常量 |
| `config_store.c` | `src/config_store.rs` | NVS blob（260 字节，与 C 版逐字节兼容） |
| `mac_store.c` | `src/mac_store.rs` | NVS blob（341 字节，与 C 版逐字节兼容） |
| `device_info.c` | `src/device_info.rs` | FFI |
| `wol.c` | `src/wol.rs` | `std::net::UdpSocket` + netif FFI |
| `chip_temp.c` | `src/chip_temp.rs` | temperature_sensor FFI |
| `time_sync.c` | `src/time_sync.rs` | `EspSntp`（esp-idf-svc 高层封装）+ 时区 setenv |
| `watchdog.c` | `src/watchdog.rs` | esp_task_wdt FFI |
| `led_status.c` | `src/led_status.rs` | `PinDriver`（esp-idf-hal）+ Condvar 事件驱动线程 |
| `wifi_prov.c` | `src/wifi_prov.rs` | network_provisioning FFI（P3 已验证） |
| `control_http.c` | `src/control_http.rs` | `EspHttpServer`（专属线程持有；index.html include_str 内嵌） |
| `control_mqtt.c` | `src/control_mqtt.rs` | `EspMqttClient::new` + 事件循环线程（阻塞 `connection.next()`） |
| `app_main.c` | `src/main.rs` | 编排入口 |
| （status 快照辅助） | `src/status.rs` | 状态/配置/地址簿的 `serde` 视图（JSON） |
| （netif/wifi 查询） | `src/netinfo.rs` | netif/wifi FFI |
| （新增，无 C 对应） | `src/debug_uart.rs` | UART1 → GPIO43/44 命令口（收字节拼行） |
| （新增，无 C 对应） | `src/serial_cmd.rs` | 串口命令分发（help / status / wifi-clear） |

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
cd /d/WakeManager   # 项目根
cargo build                                    # rust-toolchain.toml 自动选 esp 工具链；
                                               # 首次构建会完整编译 ESP-IDF（10~20 分钟）
cargo espflash flash --monitor                 # 自定义分区表已编进构建（见下），无需额外参数
```

> **分区表**：`partitions.csv`（仓库根）（factory 3MB）通过 sdkconfig.defaults 的
> `CONFIG_PARTITION_TABLE_CUSTOM=y` + `CONFIG_PARTITION_TABLE_CUSTOM_FILENAME`
> **编进了构建**。实测 esp-idf-sys 的限制在于**相对路径会被解析到 cargo 的
> OUT_DIR**，`CUSTOM_FILENAME` 必须是绝对路径——当前 sdkconfig.defaults 写的是
> CMake 的 `"$ENV{PROJECT_DIR}/partitions.csv"`（esp-idf-sys 构建时会把
> `PROJECT_DIR` 环境变量设为 Cargo 工作区根目录，因此换机器/路径无需修改）。
> 注意不要写 `${PROJECT_PATH}`：该 CMake 变量只在 `__project_info()` 作用域内
> 存在，生成分区表路径时它是空的，会退化成 `/partitions.csv`。
> 仓库根的 `partitions.csv` 仅作历史参考，权威文件是 `partitions.csv`（仓库根）。

> **Windows 构建目录（本机环境要求）**：esp-idf-sys 在 Windows 上要求
> canonicalize 后的 OUT_DIR ≤ 88 字符，因此 `target/` 实际是一个指向短路径
> （如 `D:\espbuild`）的 **NTFS 目录联接**——产物在项目路径下可见可用，
> 路径检查时又解析回短位置。换机器重建：
> `powershell "New-Item -ItemType Junction -Path D:\WakeManager\target -Target D:\espbuild"`。

## 串口命令（板载 USB 调试口）

板载 USB（立创 ESP32S3R8N8 的 Type-C）下行引出两个口，命令走其中的 **CH340K**：

- **命令口**：`/dev/cu.wchusbserial*`（macOS 用 `cu.*`；`tty.*` 要等 DCD，打不开）
  —— 115200 8N1，GPIO43(TX)/GPIO44(RX)，`debug_uart.rs` 收字节按 CR/LF 拼行 →
  `serial_cmd.rs` 分发，响应回写同一串口。
- **控制台 log 口**：`/dev/cu.usbmodem*`（原生 USB-Serial-JTAG 次级控制台镜像），
  `cargo espflash flash --monitor` 看这里；收到的命令原文也以 debug 级别打到这。

命令以 CR/LF 或 LF 结尾（上限 256 字节，超长丢弃并提示一次）；空行忽略，
未知命令提示后返回菜单：

| 命令 | 别名 | 作用 |
| --- | --- | --- |
| `help` | `cmd` / `menu` / `?` | 返回命令菜单 |
| `status` | `st` | 设备状态文本（与 `/api/status` 同一份数据）：id/ip/ssid/rssi/heap/mqtt/broadcast/macs/temp/time |
| `wifi-clear` | `reset-wifi` | 清除 NVS 里的 Wi-Fi 配置（SSID/密码）→ 回复确认 → 300ms 后重启 → 开机检测到未配网，自动重新进入 BLE 配网（App 扫日志里的 QR / PoP）。**破坏性操作，设备需重新配网才能联网** |

示例（在串口终端里发 `help` / `status`）：

```
> help
==== WakeManager 串口命令 ====
help         显示本菜单
status       查看设备状态
wifi-clear   清除 Wi-Fi 配置并重新进入配网模式（设备将重启）

> status
id=441BF681C1F4
ip=172.16.9.57
ssid=Wiseman
rssi=-60 dBm
...
```

`wifi-clear` 的实现路径：配网管理器判定"已配网"只看 STA 配置 SSID 是否非空
（network_provisioning `manager.c` 的 `network_prov_mgr_is_wifi_provisioned`），
清配置就是 `esp_wifi_restore()`。所以命令走"清 NVS + 重启"复用**开机配网路径**，
不在运行时重新拉起配网管理器（此时 mgr 已 deinit、BLE 内存已释放，重启最干净）。

加新命令：`serial_cmd.rs` 的 `handle()` match 加分支 + `MENU` 加一行即可。

## VS Code

推荐扩展（打开仓库会自动提示安装，见 `.vscode/extensions.json`）：
**rust-analyzer**（代码智能主体）、**ESP-IDF 扩展**（KConfig 高亮 / Monitor / 调试）、
**Wokwi 扩展**（按 `**/wokwi.toml` 递归查找，直接读 `wokwi/wokwi.toml` 跑模拟）。

- **构建 → 烧录 + 监视**就两步：`cargo build` 后跑
  `cargo espflash flash --monitor`（或直接 `cargo run`，runner 已配好 =
  构建 + 烧录 + 监视一体）。VS Code 里可用 `.vscode/tasks.json` 的
  "构建 (cargo build)" / "烧录 + 监视 (cargo espflash)" 任务
  （Ctrl+Shift+B 构建，Terminal → Run Task… 选烧录监视）。
  注意同一时刻只能有一个进程占用串口——用插件 Monitor 就别同时在终端
  跑 `espflash --monitor`，否则会报 `Resource busy` / 检测不到端口。
- IDF 插件的 idf.py 构建 / menuconfig 按钮对 Rust 工程不适用（无根
  CMakeLists.txt，且 sdkconfig 权威文件是 `sdkconfig.defaults`）。
- **插件的 Flash / Monitor 按钮可用**（`.vscode/settings.json`，本机配置不入库）：
  Flash 走 UART（`idf.flashType: UART`），并配了
  `idf.preFlashTask: ./wokwi/mkwokwi.sh --real`——点 Flash 会**先自动
  `cargo build`（真机 features）并把 flasher_args.json / project_description.json /
  wol-rs.elf / wol-rs.bin 组装到根 `build/`**，再用 esptool 烧录；Monitor 读根
  `sdkconfig` 软链取波特率、用 `build/project_description.json` 定位
  `wol-rs.elf`（第一次点 Flash 或跑 `mkwokwi --real` 任务后即存在）。
  端口写死为 `/dev/cu.usbmodem21401`（`tty.*` 在 macOS 上等 DCD、esptool 可能打不开；
  换 USB 口后用命令面板 "ESP-IDF: Select port to use" 重选）。
- **Monitor（串口监视）可用**：插件要求工作区根有 `sdkconfig`，这里放了一个
  **软链接指向 esp-idf-sys 的 `target/.../esp-idf-sys-*/out/sdkconfig`**（该文件
  是 cargo 构建生成的完整配置，`.gitignore` 已忽略）。esp-idf-sys 升级 / 换 target
  后 hash 目录变化、软链会悬空，重新执行：
  `ln -sfn "$(ls -td target/xtensa-esp32s3-espidf/debug/build/esp-idf-sys-*/out/sdkconfig | head -1)" sdkconfig`
- ESP-IDF 扩展的 IDF 环境用 EIM 安装（`~/.espressif/v6.1/esp-idf` + v6.1 venv，
  由 `~/.espressif/tools/eim_idf.json` 提供），Windows 老机器的
  `idf.*Win` / `idf.pythonInstallPath` 残留设置在 macOS 上无效但无害。
- **USB-JTAG 源码级调试**：`cargo espflash flash` 烧录后，用调试面板的
  "wol-rs: USB-JTAG 调试（ESP32-S3）"（`.vscode/launch.json` 指向 Rust elf +
  xtensa-esp32s3-elf-gdb，`runOpenOCD` 用 `.vscode/settings.json` 的
  `idf.openOcdConfigs=board/esp32s3-builtin.cfg` 自动拉起 OpenOCD；本机路径配置
  不入库）。**调试前先停掉残留 OpenOCD**（旧实例占着 3333 端口会让新会话失败：
  `pkill -f openocd`，或看 `ps aux | grep openocd`）。注意 rust-analyzer 的
  check 与终端构建不要并发（会撞 CMake 构建锁）。

## Wokwi 本地模拟

固件可在 [Wokwi](https://wokwi.com)（云端仿真，本机 CLI 驱动）跑 smoke test。
**限制：Wokwi 不支持 BLE**，因此 `sim` feature 直接跳过配网、连接模拟 AP
`Wokwi-GUEST`（仅用于模拟构建，真机固件不受影响）：

```bash
export WOKWI_CLI_TOKEN=...                     # wokwi.com/dashboard/ci 免费获取
cd <仓库根>
cargo build --features sim                     # sim 固件（其余代码与真机一致）
./wokwi/mkwokwi.sh                             # 组装 wokwi/build/：bootloader + 自定义分区表 + 固件镜像
wokwi-cli wokwi                                # 交互式串口 + 浏览器可视化
# 自动 smoke test（串口断言）：
wokwi-cli --expect-text "WakeManager ready" --fail-text "Guru Meditation" --timeout 90000 wokwi
```

Wokwi 相关文件统一放在 `wokwi/` 子目录：`wokwi/wokwi.toml`（elf + flasher_args
入口，路径相对本文件）、`wokwi/diagram.json`
（S3 板 + GPIO48 LED + 串口终端）、`wokwi/mkwokwi.sh`（把 cargo 链接的固件 elf 经
esptool `elf2image` 转成 app 镜像，连同 bootloader/分区表组装成
`wokwi/build/flasher_args.json` 可消费的布局；注意 esp-idf-sys CMake 产物
`libespidf.bin` 不含 Rust 代码，不能直接当固件用）。
Wokwi for VS Code 扩展按 `**/wokwi.toml` 递归查找，子目录可直接识别；
`wokwi/build/` 为生成物（gitignored）。

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

- [x] `cargo build` 编译通过（0 错误、0 警告）
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
- [ ] BLE 重新配网（用串口命令 `wifi-clear` 或擦除 NVS：清掉 Wi-Fi 配置后重启即进配网模式，手机 ESP Provisioning App 扫日志里的 QR / 输 PoP）

> 注：`partitions.csv` 的 factory 分区从 0x180000 扩到 0x300000（Rust 镜像 1.8MB 超过
> 1.5MB）。

## 优化记录（2026-10-10）

在保持报文/行为与 C 版一致的前提下做了一轮代码优化（详见 `CLAUDE.md` 的「开发经验」）：

- JSON 构造改为 `serde` 派生结构体，统一放在 `src/status.rs`（`temp_c`/`time` 按需省略）。
- `control_mqtt` 改为 `EspMqttClient::new` + 专属事件循环线程，移除 `AtomicPtr` + 回调内裸 FFI。
- `control_http` 的 `EspHttpServer` 改由专属线程创建并持有，移除 `unsafe impl Send/Sync`。
- `wifi_prov` 的 `WIFI_INIT_CONFIG_DEFAULT()` 字段改用 esp-idf-sys 常量（修正了原先 `feature_caps` 硬编码为 1 的偏差）；`wait_connected()` 改用 `Condvar`。
- `device_info::get_mac()` 返回 `Result`；HTTP 表单读取校验长度；`truncate()` 按字节并保证 UTF-8 边界。

## 优化记录（2026-10-11）

第二轮优化：消除可替代的裸 FFI + 精简依赖：

- `led_status` 改用 esp-idf-hal 的 `PinDriver`（`Peripherals::take()` + GPIO48 按值
  move 进 LED 线程），模块零 unsafe；线程改为 Condvar 事件驱动（`set()`/`activity()`
  唤醒），常亮期间不再 10Hz 轮询。
- `time_sync` 改用 esp-idf-svc 的 `EspSntp`（`new_with_callback` + `SntpConf`），
  移除 `Box::leak` server 字符串 hack；句柄由 `main()` 保活（Drop 会停 SNTP）。
  仅剩 `setenv("TZ")/tzset` 与 `format_now` 的 `localtime_r` 两处 libc FFI。
- 删除完全未使用的 `embassy-time` 依赖及 esp-idf-svc 的
  `embassy-time-driver`/`embassy-sync` features（全项目无 embassy 调用点）。
- `config_store::save` / `mac_store::persist` 缓存可写 `EspNvs` 句柄
  （`OnceLock<Mutex<…>>`），不再每次写盘重建句柄。
- MQTT 分片累积器加固：`Complete` 事件先清残留（防与上条未分发数据拼接）；
  超过 255 字节的消息整体丢弃并告警（原来静默截断且不清残留）。
- `handle_wake_command` 的地址簿查询改写为 `Option` 链，移除带副作用的闭包；
  `main` 复用 `netinfo::free_heap()`；删除未调用的 `chip_temp::is_ready()`。

## 优化记录（2026-10-11 第二批：模拟测试 + 修复）

- **自定义分区表编进构建**：`CONFIG_PARTITION_TABLE_CUSTOM` +
  `CUSTOM_FILENAME="$ENV{PROJECT_DIR}/partitions.csv"` 指向 `partitions.csv`
  （仓库根）（相对路径会被解析到 cargo OUT_DIR，此即此前记录的"esp-idf-sys
  已知限制"的真实原因）；
  `CONFIG_ESPTOOLPY_FLASHSIZE_8MB=y`。espflash 烧录不再需要
  `--partition-table` 参数。
- **新增 `sim` feature（Wokwi 模拟）**：`wifi_prov` 跳过 BLE 配网管理器，
  直接 STA 连接模拟 AP `Wokwi-GUEST`；配网相关常量/函数随 feature 编译隔离。
- **修复全新 flash 首启死循环**（Wokwi 模拟发现）：全新擦除的 NVS 上
  `EspNvs::new(..., readwrite=false)` 返回 `ESP_ERR_NVS_NOT_FOUND`，
  `config_store::init`/`mac_store::init` 原样上抛导致 `abort()` 重启循环；
  现按"无存储配置，用默认值"处理。
- **修复 MQTT 事件路径死锁**（Wokwi 模拟发现，2026-10-11）：espressif/mqtt
  组件重建时经浮动版本 `1.*` 拉到 1.1.0（事件循环新架构），mqtt 任务在 API 锁内
  分发事件，事件处理路径上调用 subscribe/publish 会与 mqtt 任务互相等待死锁
  （表象：连接成功但无订阅/上线日志，平台看不到设备）。`control_mqtt` 重构为
  API 工作线程架构：client 由专属 `mqtt_api` 线程独占，事件线程与 HTTP 线程
  只通过队列投递 SubscribeAll/Publish 操作。
- 模拟验证结论：连接 → 四主题订阅 → `online` 发布全部成功；Wokwi NAT 约 25s
  断开长连接并自动重连（模拟器限制，真机不受影响）。

## 目录



```
├── Cargo.toml              # 模板骨架 + esp-idf-sys extra_components（network_provisioning/mqtt）+ sim feature
├── build.rs                # 模板自带：embuild::espidf::sysenv::output()
├── rust-toolchain.toml     # 模板自带：channel = "esp"
├── bindings.h              # bindgen 入口头文件（network_provisioning + chip_info/flash）
├── partitions.csv          # 自定义分区表（factory 3MB，编进构建）
├── wokwi/                  # Wokwi 模拟相关全部集中在此子目录
│   ├── wokwi.toml          # Wokwi 入口（elf → ../target/... + build/flasher_args.json）
│   ├── diagram.json        # Wokwi 场景：S3 板 + GPIO48 LED + 串口终端
│   ├── mkwokwi.sh          # 组装 wokwi/build/（bootloader + 分区表 + elf2image 固件镜像）
│   └── build/              # Wokwi 模拟镜像（mkwokwi.sh 生成，gitignored）
├── .cargo/config.toml      # target/ldproxy/ESP_IDF_VERSION=v6.1（本机配置，不入库）
├── sdkconfig.defaults      # 栈大小 + NimBLE + protocomm security 1 + 自定义分区表 + 8MB flash
└── src/
    ├── main.rs             # 编排入口（对应 app_main.c）
    ├── app_config.rs       # 编译期常量（app_config.h）
    ├── config_store.rs     # NVS 配置（blob 260B 与 C 逐字节兼容）
    ├── mac_store.rs        # NVS 地址簿（blob 341B 与 C 逐字节兼容）
    ├── device_info.rs      # MAC / 设备 id
    ├── netinfo.rs          # IP/掩码/SSID/RSSI/空闲堆
    ├── status.rs           # 状态/配置/地址簿的 serde 视图（JSON）
    ├── wol.rs              # magic packet（std UdpSocket）+ 子网广播推导
    ├── chip_temp.rs        # 片内温度（temperature_sensor FFI）
    ├── time_sync.rs        # SNTP（EspSntp 高层封装 + 时区 setenv）
    ├── watchdog.rs         # TWDT（esp_task_wdt FFI）
    ├── led_status.rs       # GPIO48 + 独立线程（PinDriver，Condvar 事件驱动）
    ├── debug_uart.rs       # UART1 → GPIO43/44 命令口（拼行，见「串口命令」）
    ├── serial_cmd.rs       # 串口命令分发（help / status / wifi-clear）
    ├── wifi_prov.rs        # BLE 配网（network_provisioning FFI；sim feature 直连 Wokwi-GUEST）
    ├── index.html          # 内嵌单页 Web UI（原 components/wol_http/index.html）
    ├── control_http.rs     # EspHttpServer（index.html include_str 内嵌）
    └── control_mqtt.rs     # EspMqttClient::new + 事件循环线程 + API 工作线程
```
