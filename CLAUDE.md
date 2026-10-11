# CLAUDE.md

# 提示词优化规则

**规则说明：**  
在执行任何操作之前，第一步始终是对用户的提示词进行优化，使其更加清晰和有效。

**优化原则：**

- 保持用户原始意图不变
- 仅优化表达清晰度、结构性和可执行性
- 不改变业务逻辑、技术选型或具体需求
- 优化程度：微调为主，避免大幅重写

**执行流程：**

1. **第一步：判断是否需要优化**

   - 如果用户明确要求"不优化"、"直接执行"等，跳过优化步骤
   - 如果提示词已经非常清晰具体，可以跳过优化
   - 其他情况进入优化流程

2. **第二步：优化提示词**

   - 分析用户原始提示词的核心意图
   - 识别模糊、不清晰或可能产生歧义的部分
   - 生成优化后的提示词（更清晰、更具体、更易执行）
   - 记录优化理由和主要改动点
   - 如果是需求开发类的功能，看看有没有比用户所想要的更好的解决方案，如果有列出来，并说明

3. **第三步：展示 A/B/C 选项（仅展示，不执行）**

   - **A：原始提示词** - 使用用户原始提示词执行
   - **B：优化后的提示词** - 使用优化后的提示词执行
   - **C：重新优化** - 根据反馈重新优化提示词，然后再次展示选项
   - **必须完整展示提示词内容**，不要只显示一行摘要或简化版本
   - 使用代码块或引用格式完整展示，确保用户能看到完整的提示词内容
   - **必须说明优化理由**：简要说明为什么这样优化，主要改进了哪些方面

4. **第四步：等待用户选择**

   - **必须等待用户明确选择 A、B 或 C**
   - 如果用户选择 C：
     - 询问用户希望如何调整（更具体、更简洁、关注不同方面等），或根据用户的反馈意见
     - 根据用户反馈重新优化提示词
     - 返回第三步，再次展示 A/B/C 选项
     - 可以多次迭代，直到用户选择 A 或 B
   - 在用户选择之前，**不得执行任何后续操作**
   - 不得进行代码分析、文件读取、工具调用等操作
   - 只能展示选项，等待用户交互

5. **第五步：执行操作**
   - 用户选择 A 或 B 后，使用选定的提示词执行后续操作

**重要约束：**

- ❌ 禁止在展示选项后立即继续执行
- ❌ 禁止在用户未选择时进行任何分析或工具调用
- ❌ 禁止只展示一行摘要或简化版本的提示词
- ❌ 禁止改变用户原始意图和核心需求
- ❌ 禁止过度优化导致提示词面目全非
- ✅ 必须完整展示提示词内容
- ✅ 必须说明优化理由
- ✅ 只能展示选项并明确说明"请选择 A、B 或 C"
- ✅ 用户选择后再继续执行
- ✅ 如果用户选择 C，根据反馈重新优化并再次展示选项

**用户反馈处理：**

- 如果用户选择 C（重新优化），AI 应该：

  1. 询问用户希望如何调整（更具体、更简洁、关注不同方面等），或根据用户提供的具体反馈
  2. 根据用户反馈重新优化提示词
  3. 返回第三步，再次展示 A/B/C 选项供用户选择
  4. 可以多次迭代，直到用户选择 A 或 B 为止

- ✅ 只能展示选项并明确说明"请选择 A 或 B"
- ✅ 用户选择后再继续执行

---

## 项目概述

**WakeManager** 是一个基于 **ESP-IDF v6.1** 的 ESP32-S3 网络唤醒（Wake-on-LAN）控制器。设备通过 BLE 配网接入 Wi-Fi 后，提供两种控制方式：

- **HTTP**：内置单页 Web UI + REST API，用于查看状态、立即唤醒、修改配置。
- **MQTT**：远程唤醒设备、发现设备（获取设备唯一 ID）、管理地址簿。

核心能力：

- **网络唤醒**：向指定 MAC（必传）发送 WOL 魔术包；目标广播地址自动推导，也可在发送时指定 IP 覆盖。
- **运行时配置**：所有参数经 NVS 持久化，可在网页上修改。
- **设备唯一标识**：由 Wi-Fi STA MAC 生成 12 位十六进制 ID，用于 MQTT 寻址与发现。
- **设备发现**：所有设备订阅同一个广播主题，远程无需预先知道设备 ID 即可先发现再操作。
- **BLE 配网**：基于 `espressif/network_provisioning`。

技术栈：**Rust（`esp-idf-svc` std 路线 / ESP-IDF v6.1）**，目标芯片 **ESP32-S3**。原 C 实现已移除。

## 常用命令

> 本机 ESP-IDF 未加入 PATH。Rust 工程自带 `rust-toolchain.toml`（`channel = "esp"`），
> 通过 `espup` 安装的 Xtensa 工具链构建。每个新 shell 需要先激活环境：

```bash
# 激活 ESP-IDF / Rust 环境（每条新 shell 都需要）
. ~/export-esp.sh          # LIBCLANG_PATH（bindgen 用）
. "$HOME/.cargo/env"       # cargo / rustup

# 构建（首次会完整编译 ESP-IDF，10~20 分钟；后续增量很快）
cd wol-rs
cargo build

# 烧录 + 串口监视（自定义分区表已编进构建，无需额外参数）
cargo espflash flash --monitor

# Wokwi 本地模拟（WOKWI_CLI_TOKEN 见 https://wokwi.com/dashboard/ci）
cargo build --features sim && ./mkwokwi.sh
wokwi-cli --expect-text "WakeManager ready" --fail-text "Guru Meditation" --timeout 90000 .

# 清理
cargo clean
```

生成物：`wol-rs/target/xtensa-esp32s3-espidf/debug/wol-rs`。

## 项目结构

```
WakeManager/
├── partitions.csv              # 历史副本（权威：wol-rs/partitions.csv）
├── README.md / CLAUDE.md
└── wol-rs/                     # Rust 固件工程（cargo）
    ├── Cargo.toml              # 依赖 + esp-idf-sys extra_components（mqtt / network_provisioning）+ sim feature
    ├── build.rs                # embuild::espidf::sysenv::output()
    ├── bindings.h              # bindgen 入口（network_provisioning + chip/flash info）
    ├── rust-toolchain.toml     # channel = "esp"
    ├── sdkconfig.defaults      # 主任务栈大小 + NimBLE + protocomm security 1 + 日志时间戳
    ├── .cargo/config.toml      # target / ldproxy / ESP_IDF_VERSION=v6.1 / 复用全局工具链
    └── src/
        ├── main.rs             # 编排入口（对应原 app_main.c）
        ├── index.html          # 内嵌单页 Web UI（include_str!）
        ├── app_config.rs       # 编译期常量（原 app_config.h）
        ├── config_store.rs     # NVS 配置（blob 260B）
        ├── mac_store.rs        # NVS 地址簿（blob 341B）
        ├── device_info.rs      # MAC / 设备 id
        ├── netinfo.rs          # IP / 掩码 / SSID / RSSI / 空闲堆
        ├── status.rs           # 状态/配置/地址簿的 serde 视图（JSON 构造）
        ├── wol.rs              # 魔术包（std UdpSocket）+ 子网广播推导
        ├── chip_temp.rs        # 片内温度（temperature_sensor FFI）
        ├── time_sync.rs        # SNTP（EspSntp 高层封装 + 时区 setenv）
        ├── watchdog.rs         # TWDT（esp_task_wdt FFI）
        ├── led_status.rs       # GPIO48 + 独立线程（PinDriver，Condvar 事件驱动）
        ├── wifi_prov.rs        # BLE 配网（network_provisioning FFI）
        ├── control_http.rs     # EspHttpServer（专属线程持有）+ 内嵌 index.html
        └── control_mqtt.rs     # EspMqttClient + 事件循环线程
```

托管组件（由 `Cargo.toml` 的 `extra_components` 声明）：`espressif/mqtt`、`espressif/network_provisioning`（后者还依赖 `espressif/json_generator`）。

## 配置（config_store）

运行时配置以单个 blob 存于 NVS 命名空间 `wm_cfg`（key `cfg`），字段定义见 `wol-rs/src/config_store.rs` 的 `WmConfig`；编译期默认值在 `wol-rs/src/app_config.rs`。blob 布局与原 C 版 `wm_config_t` **逐字节兼容（260 字节）**。

| 字段 | 说明 |
| --- | --- |
| `wol_udp_port` | 魔术包 UDP 端口（默认 9） |
| `mqtt_enable` | 是否启用 MQTT |
| `mqtt_uri` / `mqtt_username` / `mqtt_password` | Broker 与凭据 |

> **无默认目标 MAC、无广播地址配置项。** 唤醒时 `mac` 必传；目标地址按 运行时 `ip` → 地址簿中该 MAC 的 IP → 自动推导的子网广播 依次决定。

> **MQTT 主题不再是运行时可配置项**，全部固定为 `app_config.rs` 中的常量：`WOL_MQTT_CMD_TOPIC`、`WOL_MQTT_BROADCAST_TOPIC`、`WOL_MQTT_STATUS_TOPIC`、`WOL_MQTT_MAC_TOPIC`、`WOL_MQTT_MAC_GET_TOPIC`。

## MAC/IP 地址簿（mac_store）

`wol-rs/src/mac_store.rs`：

- 最多存 **10 组** MAC + IP，整体以单个 blob 存于 NVS 命名空间 `wm_mac`（key `list`），互斥锁保护。blob 布局与原 C 版 **逐字节兼容（341 字节）**。
- MAC 统一规范化为 `AA:BB:CC:DD:EE:FF`；IP 可为空（唤醒时走自动推导的广播）。
- API：`mac_store::init()`、`count()`、`get()`、`set(mac, ip)`、`remove(mac)`、`clear()`、`get_ip(mac)`。
- 唤醒命令只给 `mac` 时，会自动用地址簿里该 MAC 存的 IP 作为目标（未命中则用默认广播）。

## 芯片温度（chip_temp）

`chip_temp.rs`：封装 `esp_driver_tsens` 的内部温度传感器，`init()`（开机初始化一次）、`read()`。

## 时间同步（time_sync）

`time_sync.rs`：Wi-Fi 拿到 IP 后调用 `time_sync::start()`，通过 SNTP 校时（esp-idf-svc 的 `EspSntp` 封装，句柄由 `main()` 保活）；服务器与 POSIX 时区分别由 `NTP_SERVER`、`TIME_TIMEZONE`（默认 `pool.ntp.org` / `CST-8`）固定。`is_synced()` 判断是否已同步，`format_now()` 输出本地时间字符串。

## 看门狗（watchdog）

`watchdog.rs`：`init()` 配置并开启任务看门狗（`esp_task_wdt`，超时 `WATCHDOG_TIMEOUT_MS`，`WATCHDOG_TRIGGER_PANIC` 决定超时是否复位），监控各核 idle 任务；主任务在 Wi-Fi 连接后 `add_current_task()` 订阅并在循环中 `feed()`。

## 网络唤醒（WOL）行为

`wol.rs`：

- 魔术包 = 6 字节 `0xFF` + 目标 MAC 重复 16 次（共 102 字节），UDP 广播发送（`std::net::UdpSocket`）。
- **目标 MAC 必传**；目标地址解析顺序：运行时的 `ip` → 地址簿中该 MAC 存的 IP → 自动推导的子网广播。
- **广播地址自动推导**：`directed broadcast = (ip & netmask) | ~netmask`，用 STA 网卡 IP 与掩码计算，支持任意前缀长度；网卡无 IP 时回退 `255.255.255.255`。
- API：`wol::send_to(mac, dest_ip)`、`send_str_to(mac_str, dest_ip)`、`get_default_broadcast()`。

## HTTP 接口

`control_http.rs`（`EspHttpServer`，由专属线程创建并持有；默认端口 80，`index.html` 用 `include_str!` 内嵌）：

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/` | 内嵌单页 UI |
| GET | `/api/status` | 状态 JSON：`id`、`ip`、`ssid`、`rssi`、`heap`、`mqtt_connected`、`broadcast`、`mac_count`、`temp_c`、`time` |
| GET | `/api/config` | 当前配置 JSON |
| POST | `/api/config` | 保存配置（`x-www-form-urlencoded`），随后重载 MQTT |
| GET | `/api/wake` | 发送魔术包，**`?mac=<MAC>` 必传**，可选 `?ip=<IP>` |
| GET | `/api/mac` | 地址簿 JSON：`{"count":n,"macs":[{"mac":...,"ip":...}]}` |

## MQTT 协议

`control_mqtt.rs`。设备 ID（`id`）= Wi-Fi STA MAC 的 12 位大写十六进制，如 `A1B2C3D4E5F6`。

**订阅四个主题**（主题全部固定，`<id>` = 设备 ID）：

1. **广播主题**（共享，`WOL_MQTT_BROADCAST_TOPIC` = `wakemanager/broadcast`）
   - 所有设备都订阅；远程在不知道任何设备 ID 时用它做**设备发现**。
   - 收到 `{"cmd":"status"}`（或缺省 `cmd`）→ 立即把自身状态（含 `id`）发布到状态主题。
   - **不接受唤醒命令**。

2. **唤醒主题**（每设备，`WOL_MQTT_CMD_TOPIC` + `/<id>`，如 `wakemanager/cmd/A1B2C3D4E5F6`）
   - 唤醒命令：`{"cmd":"wake","mac":"AA:BB:CC:DD:EE:FF","ip":"192.168.1.255"}`
   - **`mac` 必传**；`ip` 可选，省略时优先用地址簿中该 MAC 存的 IP、否则用自动推导的广播地址。

3. **地址簿设置主题**（每设备，`WOL_MQTT_MAC_TOPIC` + `/<id>`，如 `wakemanager/mac/A1B2C3D4E5F6`）
   - 新增/更新：`{"cmd":"set","mac":"AA:BB:CC:DD:EE:FF","ip":"192.168.1.5"}`（`add`/`update` 同义）
   - 删除：`{"cmd":"del","mac":"AA:BB:CC:DD:EE:FF"}`（`delete`/`remove` 同义）
   - 清空：`{"cmd":"clear"}`
   - 结果发布到状态主题（`ok: mac stored` / `error: address book full` 等）。

4. **地址簿获取主题**（每设备，`WOL_MQTT_MAC_GET_TOPIC` + `/<id>`，如 `wakemanager/mac/get/A1B2C3D4E5F6`）
   - 收到 `{"cmd":"get"}`（或空消息）→ 把完整列表发布到状态主题。
   - 列表 JSON：`{"id":"A1B2C3D4E5F6","cmd":"mac_list","count":2,"macs":[{"mac":"AA:BB:...","ip":"192.168.1.5"},...]}`

**发布：** 命令结果、发现应答、地址簿列表发布到 `WOL_MQTT_STATUS_TOPIC`（`wakemanager/status`）。芯片温度仅在按需发布的状态快照（`temp_c` 字段）中返回。

**指示灯：** `led_status` 在 MQTT **收发数据**时使 LED 快速闪烁约 **2 秒**，随后自动恢复到闪烁前由 `set()` 设定的状态（连接中慢闪 / 已连接常亮）。

**典型流程（远程不知道设备 ID）：**

```
1) 远程 publish  广播主题        {"cmd":"status"}
2) 各设备 publish 状态主题        {... "id":"A1B2C3D4E5F6" ...}   ← 取得设备 ID
3) 远程 publish  wakemanager/cmd/A1B2C3D4E5F6   {"cmd":"wake","mac":"..."}
4) 远程 publish  wakemanager/mac/A1B2C3D4E5F6   {"cmd":"set","mac":"...","ip":"..."}
5) 远程 publish  wakemanager/mac/get/A1B2C3D4E5F6  {"cmd":"get"}
```

**命令格式仅支持 JSON。**

## 代码规范

- **一模块一文件**：每个功能放在 `wol-rs/src/<name>.rs`，模块顶部用 `//!` 写明对应原 C 组件。
- 日志统一用 `log` crate（`log::info!` / `warn!` / `error!`）；注意 `esp_idf_sys::*` glob 里也有 `log`，需要时写 `::log::info!`。
- 错误处理：内部函数返回 `Result`，最外层 `main` 对致命错误 `std::process::abort()`（等价 C 的 `ESP_ERROR_CHECK`）。
- 配置读写一律走 `config_store::get/save`；地址簿读写一律走 `mac_store::*`（内部加锁），不要直接读 NVS。
- JSON 统一用 `serde` 派生结构体 + `serde_json`（替代 cJSON / 手写 sb_add）；状态、配置、地址簿的视图集中在 `src/status.rs`，字段顺序与原 C 输出一致。

## 开发经验 / 注意事项

- **NVS blob 兼容**：`config_store`（260B）与 `mac_store`（341B）刻意保持与原 C `repr(C)` 结构逐字节一致；修改字段布局会使旧配置失效。
- **ffi / 宏不可见**：`TEMPERATURE_SENSOR_CONFIG_DEFAULT()`、`NETWORK_PROV_EVENT_HANDLER_*` 等含复合字面量的宏 bindgen 不可见，需手工构造结构体（见 `wifi_prov.rs` / `chip_temp.rs`）。`WIFI_INIT_CONFIG_DEFAULT()` 同样不可见，但其字段值**全部取自 esp-idf-sys 生成的 `WIFI_*` / `CONFIG_*` 常量**（与 esp-idf-svc 内部一致），改 menuconfig 会自动跟随，无需手工同步。
- **bindgen 命名**：枚举常量名通常去掉了类型 `_t` 后缀（如 `network_prov_security_NETWORK_PROV_SECURITY_1`）；`ssize_t` 绑定为 `isize`。以 `_t_` 开头的 bindgen 常量在 `match` 中触发 `non_upper_case_globals` 警告，可在函数上加 `#[allow(non_upper_case_globals)]`。
- **MQTT 生命周期**：`control_mqtt.rs` 用 `EspMqttClient::new` 得到 `(client, connection)`：connection 交给专属事件线程用阻塞的 `connection.next()` 驱动事件循环；client 交由**专属 API 工作线程（`mqtt_api`）独占**，事件线程与 HTTP 线程只通过 `API_TX` 队列投递 SubscribeAll/Publish 操作。`stop()` 先关队列 → 工作线程退出并 drop client → 事件通道关闭 → join 两个线程。状态主题统一取自 `app_config`。**事件线程栈必须 ≥8192**：事件处理含 serde_json 与大 FFI 栈结构（`wifi_ap_record_t` 约 800B），4KB 会触发栈金丝雀整机遇测复位（表象为发现命令无应答）。**client id 每设备唯一**（`wakemanager-<设备id>`），固定 ID 会被 broker 互踢导致发现时好时坏。
- **MQTT API 锁死锁（espressif/mqtt 1.1.0，2026-10-11 Wokwi 模拟发现）**：mqtt 任务循环在 API 锁内分发事件（`MQTT_API_LOCK → run_event_loop → dispatch → svc 回调 share() 阻塞等事件线程处理完，锁全程不放`）。因此**事件处理路径上（含 Connected 分支）调用任何 client API（subscribe/publish）都会互相等待死锁**——表象为连接成功但订阅/上线日志缺失、设备在平台上看不到。修复：所有 client API 走 `mqtt_api` 工作线程。诱因是 `Cargo.toml` 的 `espressif/mqtt version = "1.*"` 浮动版本在重建时拉到 1.1.0（事件循环新架构）；旧组件（10-10 真机验证时的解析结果）无此问题。
- **Wokwi 模拟网络限制**：不支持 BLE（`sim` feature 跳过配网）；出站 TCP/DNS/UDP(NTP) 正常，但长连接约 25s 被 NAT 断开（`MQTT error: ESP_FAIL → Disconnected → 自动重连`循环），真机不受影响。
- **HTTP 服务器持有**：`EspHttpServer` 非 `Send`（内含裸指针），`control_http.rs` 在专属线程内创建并持有它（不跨线程移动），因此无需 `unsafe impl Send/Sync`；`start()` 通过 channel 等待线程回报启动结果。
- **等待 IP**：`wifi_prov::wait_connected()` 用 `Mutex<bool> + Condvar` 阻塞等待（事件回调里 `notify_all`），不再轮询。
- **MQTT 数据不是 NUL 结尾**：`EventPayload::Received` 的 `topic`/`data` 需按长度/UTF-8 处理，避免越界。
- **看门狗**：`watchdog::init()` 会 `reconfigure`（IDF 默认已 `CONFIG_ESP_TASK_WDT_INIT` 自动初始化）；等待配网期间不要提前把主任务加入看门狗，应在开始循环喂狗后再 `add_current_task()`。
- **SNTP**：现在走 esp-idf-svc 的 `EspSntp`（server 字符串生命周期由句柄托管，`main()` 保活）；时区仍用 `setenv("TZ", ...)` + `tzset()`（0.53 的 `SntpConf` 无 timezone 字段）。
- **分区表**：`wol-rs/partitions.csv`（factory 0x300000）通过 `CONFIG_PARTITION_TABLE_CUSTOM=y` 编进构建；esp-idf-sys 的限制是**相对路径被解析到 cargo OUT_DIR**，`CUSTOM_FILENAME` 必须写绝对路径（换机器需同步改 sdkconfig.defaults）。flash size 相应设为 4MB。
- **Windows 构建目录**：esp-idf-sys 要求 canonicalize 后的 OUT_DIR ≤ 88 字符；`wol-rs/target` 是指向短路径（`D:\espbuild`）的 NTFS junction——产物在项目内可见、检查时解析回短路径。换机器需重建 junction（`New-Item -ItemType Junction`），不能用 subst（canonicalize 会还原成原长路径）。
- **NVS 全新 flash 首启**：只读 `EspNvs::new(..., false)` 在命名空间不存在时返回 `ESP_ERR_NVS_NOT_FOUND`，`config_store`/`mac_store` 的 `init` 必须按"无存储配置"处理而非上抛（此 bug 由 Wokwi 模拟发现——真机全擦后首启会 abort 重启循环）。
- **Wokwi 模拟**：不支持 BLE → `sim` feature 跳过配网直连 `Wokwi-GUEST`；`wokwi.toml` 不要设 `gdbServerPort`（会暂停仿真等 GDB）；esp-idf-sys 的 CMake 产物 `libespidf.bin` 不含 Rust 代码，模拟镜像必须用 cargo elf 经 `esptool elf2image` 转换（`mkwokwi.sh`）。
- **日志时间戳**：`CONFIG_LOG_TIMESTAMP_SOURCE_SYSTEM=y`，串口日志显示 `HH:MM:SS.sss`（本地时间）；SNTP 校时前为 epoch。
- **`unsafe` 边界**：FFI 调用集中在各模块，回调用 `unsafe extern "C" fn`；`EspHttpServer` 与 MQTT 客户端都由各自专属线程持有，避免跨线程移动非 `Send` 类型。