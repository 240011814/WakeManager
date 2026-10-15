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
- **MQTT**：远程唤醒设备、发现设备（获取设备唯一 ID）、管理地址簿、触发 OTA 升级。

核心能力：

- **网络唤醒**：向指定 MAC（必传）发送 WOL 魔术包；目标广播地址自动推导，也可在发送时指定 IP 覆盖。
- **运行时配置**：所有参数经 NVS 持久化，可在网页上修改。
- **设备唯一标识**：由 Wi-Fi STA MAC 生成 12 位十六进制 ID，用于 MQTT 寻址与发现。
- **设备发现**：所有设备订阅同一个广播主题，远程无需预先知道设备 ID 即可先发现再操作。
- **近场人体感知**：Wi-Fi CSI 近场遮挡近似探测 ~1 米内是否有人，状态变化时发 MQTT 通知（`csi/notify`）。
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
cd D:/WakeManager   # 项目根
cargo build

# 烧录 + 串口监视（自定义分区表已编进构建，无需额外参数）
cargo espflash flash --monitor

# Wokwi 本地模拟（WOKWI_CLI_TOKEN 见 https://wokwi.com/dashboard/ci）
cargo build --features sim && ./wokwi/mkwokwi.sh
wokwi-cli --expect-text "WakeManager ready" --fail-text "Guru Meditation" --timeout 90000 wokwi

# 清理
cargo clean
```

生成物：`target/xtensa-esp32s3-espidf/debug/wol-rs`。

## 项目结构

项目根即 cargo 工程根（`D:\WakeManager`）：

```
WakeManager/
├── Cargo.toml                  # 依赖 + esp-idf-sys extra_components（mqtt / network_provisioning）+ sim feature
├── build.rs                    # embuild::espidf::sysenv::output() + 生成 clangd 编译数据库
├── bindings.h                  # bindgen 入口（network_provisioning + chip/flash info）
├── rust-toolchain.toml         # channel = "esp"
├── sdkconfig.defaults          # 栈大小 + NimBLE + protocomm security 1 + 自定义分区表（$ENV{PROJECT_DIR}）+ 8MB flash
├── partitions.csv              # 自定义分区表（OTA 双槽 ota_0/ota_1，编进构建）
├── wokwi/                      # Wokwi 模拟相关统一放此子目录
│   ├── wokwi.toml / diagram.json # 模拟入口与场景（elf → ../target/...，firmware → build/flasher_args.json）
│   ├── mkwokwi.sh              # 组装 wokwi/build/（bootloader + 分区表 + elf2image 固件镜像）
│   └── build/                  # 模拟镜像（mkwokwi.sh 生成，gitignored）
├── .cargo/config.toml          # target / ldproxy / ESP_IDF_VERSION=v6.1（本机配置，不入库）
├── target/                     # 构建产物（NTFS junction → D:\espbuild，见「开发经验」）
└── src/
    ├── main.rs                 # 编排入口（对应原 app_main.c）
    ├── index.html              # 内嵌单页 Web UI（include_str!）
    ├── app_config.rs           # 编译期常量（原 app_config.h）
    ├── app_log.rs              # 自定义日志前端（EspLogger 不认 SYSTEM_FULL 时间戳 cfg）
    ├── config_store.rs         # NVS 配置（blob 260B）
    ├── mac_store.rs            # NVS 地址簿（blob 341B）
    ├── device_info.rs          # MAC / 设备 id
    ├── netinfo.rs              # IP / 掩码 / SSID / RSSI / 空闲堆
    ├── status.rs               # 状态/配置/地址簿的 serde 视图（JSON 构造）
    ├── wol.rs                  # 魔术包（std UdpSocket）+ 子网广播推导
    ├── chip_temp.rs            # 片内温度（temperature_sensor FFI）
    ├── time_sync.rs            # SNTP（EspSntp 高层封装 + 时区 setenv）
    ├── watchdog.rs             # TWDT（esp_task_wdt FFI）
    ├── led_status.rs           # GPIO48 + 独立线程（PinDriver，Condvar 事件驱动）
    ├── touch.rs                # 触摸 T1/GPIO1 轻触闪灯 + T7/GPIO7 长按 20s 清 Wi-Fi 进配网（touch_pad FFI，轮询 + 基线校准）
    ├── debug_uart.rs           # UART1 → GPIO43/44 命令口（收字节按 CR/LF 拼行）
    ├── serial_cmd.rs           # 串口命令分发（help / status / wifi-clear）
    ├── wifi_prov.rs            # BLE 配网（network_provisioning FFI；sim 直连 Wokwi-GUEST）
    ├── ota.rs                  # OTA：GitHub Releases 版本检查 + 流式下载升级（esp_ota 双槽）
    ├── control_http.rs         # EspHttpServer（专属线程持有）+ 内嵌 index.html
    ├── control_mqtt.rs         # EspMqttClient + 事件循环线程 + API 工作线程
    └── csi_presence.rs         # Wi-Fi CSI 近场人体感知（~1m 有人/无人，状态变化发 MQTT 通知）
```

托管组件（由 `Cargo.toml` 的 `extra_components` 声明）：`espressif/mqtt`、`espressif/network_provisioning`（后者还依赖 `espressif/json_generator`）。

## 配置（config_store）

运行时配置以单个 blob 存于 NVS 命名空间 `wm_cfg`（key `cfg`），字段定义见 `src/config_store.rs` 的 `WmConfig`；编译期默认值在 `src/app_config.rs`。blob 布局与原 C 版 `wm_config_t` **逐字节兼容（260 字节）**。

| 字段 | 说明 |
| --- | --- |
| `wol_udp_port` | 魔术包 UDP 端口（默认 9） |
| `mqtt_enable` | 是否启用 MQTT |
| `mqtt_uri` / `mqtt_username` / `mqtt_password` | Broker 与凭据 |

> **无默认目标 MAC、无广播地址配置项。** 唤醒时 `mac` 必传；目标地址按 运行时 `ip` → 地址簿中该 MAC 的 IP → 自动推导的子网广播 依次决定。

> **MQTT 主题不再是运行时可配置项**，全部固定为 `app_config.rs` 中的常量：`WOL_MQTT_CMD_TOPIC`、`WOL_MQTT_BROADCAST_TOPIC`、`WOL_MQTT_STATUS_TOPIC`、`WOL_MQTT_MAC_TOPIC`、`WOL_MQTT_MAC_GET_TOPIC`、`WOL_MQTT_OTA_TOPIC`。

## MAC/IP 地址簿（mac_store）

`src/mac_store.rs`：

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

## Wi-Fi CSI 近场人体感知（csi_presence）

`csi_presence.rs`（需 sdkconfig `CONFIG_ESP_WIFI_CSI_ENABLED=y`，已在 sdkconfig.defaults 开启）：探测 **~1 米内是否有人**，状态变化时发 MQTT 通知。

- **原理（近场遮挡近似，非精确测距）**：STA 模式下 Wi-Fi 每收到一帧（主要是 AP beacon/下行帧，约 2~10 帧/秒）回调一次 CSI 子载波 I/Q；人体贴近（约 1 米内）遮挡/改变首段传播路径，使接收幅度相对「无人基线」显著衰减 →「窗口幅度均值比基线低 ≥ `CSI_ENTER_DROP`（10%）」判定有人。CSI 无法真正测距，「1 米」是近场遮挡效应的近似，阈值按现场（AP 距离/摆放）微调（`app_config.rs`）。
- **数据流**：CSI 回调（Wi-Fi 任务上下文，只做逐子载波功率累加 + 入环形缓冲，禁日志/分配）→ `csi_det` 检测线程（500ms 评估窗口均值/标准差）→ 状态机（迟滞 `CSI_ENTER/LEAVE_DROP` + 连续达标防抖 2s/6s）→ 状态变化时发 `csi/notify`。断流 30s 告警一次（Wi-Fi 空闲或 CSI 被关）。
- **基线自适应**：仅「无人」状态自适应（τ≈25s）吸收环境慢漂移；「有人」状态冻结基线（否则久坐的人会被吸进基线导致漏检）。开机校准假设现场无人——校准时有人会污染基线，人走后 ~30s 自动修正，**首个「离开」事件可能漏报**；环境永久变化发生在有人期间可能造成状态粘滞 → 串口 `csi recal` 强制重新校准。
- **通知消息**（`WOL_MQTT_CSI_NOTIFY_TOPIC` = `csi/notify`，固定主题不追加设备 id，消息内含 `id`，QoS 1）：
  `{"id":"...","cmd":"csi","event":"init|change","state":"absent|present","drop":0.13,"std":0.02,"baseline":52.3,"amp":45.5,"frames":256}`。
  开机校准完成后先发一条 `event:"init"`（state 按校准假设 = absent），之后有人↔无人各发一条 `event:"change"`；`drop` 正 = 幅度低于基线 = 有人贴近。
- 串口 `csi` 查看实时量（state/baseline/amp/std/drop/rssi/frames），`csi recal` 重新校准基线。感知状态同时并入 `DeviceStatus.presence`（`/api/status`、MQTT 状态快照、串口 `status`、网页 UI 共用同一份数据）。

## HTTP 接口

`control_http.rs`（`EspHttpServer`，由专属线程创建并持有；默认端口 80，`index.html` 用 `include_str!` 内嵌）：

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/` | 内嵌单页 UI |
| GET | `/api/status` | 状态 JSON：`id`、`ip`、`ssid`、`rssi`、`heap`、`mqtt_connected`、`broadcast`、`mac_count`、`presence`（1 米内有人/无人）、`temp_c`、`time`、`version`、`ota`（OTA 状态机） |
| GET | `/api/config` | 当前配置 JSON |
| POST | `/api/config` | 保存配置（`x-www-form-urlencoded`），随后重载 MQTT |
| GET | `/api/wake` | 发送魔术包，**`?mac=<MAC>` 必传**，可选 `?ip=<IP>` |
| GET | `/api/mac` | 地址簿 JSON：`{"count":n,"macs":[{"mac":...,"ip":...}]}` |
| GET | `/api/ota/check` | 触发 OTA 版本检查（异步；结果经 `/api/status` 的 `ota` 字段轮询） |
| POST | `/api/ota` | 触发 OTA 升级：下载 Release 的 `wol-rs.bin` → 写入非活动槽 → 校验 → 重启 |

## MQTT 协议

`control_mqtt.rs`。设备 ID（`id`）= Wi-Fi STA MAC 的 12 位大写十六进制，如 `A1B2C3D4E5F6`。

**订阅五个主题**（主题全部固定，`<id>` = 设备 ID）：

1. **广播主题**（共享，`WOL_MQTT_BROADCAST_TOPIC` = `broadcast`）
   - 所有设备都订阅；远程在不知道任何设备 ID 时用它做**设备发现**。
   - 收到 `{"cmd":"status"}`（或缺省 `cmd`）→ 立即把自身状态（含 `id`）发布到状态主题。
   - **不接受唤醒命令**。

2. **唤醒主题**（每设备，`WOL_MQTT_CMD_TOPIC` + `/<id>`，如 `cmd/A1B2C3D4E5F6`）
   - 唤醒命令：`{"cmd":"wake","mac":"AA:BB:CC:DD:EE:FF","ip":"192.168.1.255"}`
   - **`mac` 必传**；`ip` 可选，省略时优先用地址簿中该 MAC 存的 IP、否则用自动推导的广播地址。

3. **地址簿设置主题**（每设备，`WOL_MQTT_MAC_TOPIC` + `/<id>`，如 `mac/A1B2C3D4E5F6`）
   - 新增/更新：`{"cmd":"set","mac":"AA:BB:CC:DD:EE:FF","ip":"192.168.1.5"}`（`add`/`update` 同义）
   - 删除：`{"cmd":"del","mac":"AA:BB:CC:DD:EE:FF"}`（`delete`/`remove` 同义）
   - 清空：`{"cmd":"clear"}`
   - 结果发布到状态主题（`ok: mac stored` / `error: address book full` 等）。

4. **地址簿获取主题**（每设备，`WOL_MQTT_MAC_GET_TOPIC` + `/<id>`，如 `mac/get/A1B2C3D4E5F6`）
   - 收到 `{"cmd":"get"}`（或空消息）→ 把完整列表发布到状态主题。
   - 列表 JSON：`{"id":"A1B2C3D4E5F6","cmd":"mac_list","count":2,"macs":[{"mac":"AA:BB:...","ip":"192.168.1.5"},...]}`

5. **OTA 升级主题**（每设备，`WOL_MQTT_OTA_TOPIC` + `/<id>`，如 `ota/A1B2C3D4E5F6`）
   - 检查更新：`{"cmd":"check"}` → 回 `ok: version check started`，完成后状态快照 `ota` 字段为 `update_available`/`idle`（`latest` = 新版本短 SHA）。
   - 开始升级：`{"cmd":"upgrade"}` → 回 `ok: upgrade started, device will reboot when done` 或 `error: 请先检查更新`；下载进度在状态快照 `ota.progress`，完成后设备自动重启。
   - 查状态：`{"cmd":"status"}` → 发布设备状态快照（含 `version` 与 `ota` 状态机）。
   - check/upgrade 只做触发（检查/下载在 OTA 专属 32KB 栈线程跑），进行中重复触发会回 `error: OTA 正在进行中`。

**发布：** 命令结果、发现应答、地址簿列表发布到 `WOL_MQTT_STATUS_TOPIC`（`status`）。芯片温度仅在按需发布的状态快照（`temp_c` 字段）中返回。CSI 近场感知的状态变化通知发布到 `WOL_MQTT_CSI_NOTIFY_TOPIC`（`csi/notify`，见「Wi-Fi CSI 近场人体感知」）。

**指示灯：** `led_status` 在 MQTT **收发数据**时使 LED 快速闪烁约 **2 秒**，随后自动恢复到闪烁前由 `set()` 设定的状态（连接中慢闪 / 已连接常亮）。

**典型流程（远程不知道设备 ID）：**

```
1) 远程 publish  广播主题        {"cmd":"status"}
2) 各设备 publish 状态主题        {... "id":"A1B2C3D4E5F6" ...}   ← 取得设备 ID
3) 远程 publish  cmd/A1B2C3D4E5F6   {"cmd":"wake","mac":"..."}
4) 远程 publish  mac/A1B2C3D4E5F6   {"cmd":"set","mac":"...","ip":"..."}
5) 远程 publish  mac/get/A1B2C3D4E5F6  {"cmd":"get"}
```

**命令格式仅支持 JSON。**

## OTA 固件升级

`ota.rs`（更新源：GitHub Releases 的 `wol-rs.bin` 资产，由 CI 每次构建成功后推送到滚动 Release）：

- **版本注入**：build.rs 读环境变量 `WOL_VERSION`（CI 传 **commit 短 SHA** 如 `7087f95`，本地兜底 `"dev"`）→ `app_config::VERSION`；`/api/status` 的 `version` 字段暴露。**不打版本 tag**：Release 是固定 tag=`latest` 的滚动发布，版本即 commit 短 SHA。
- **状态机**（全局 Mutex，`/api/status` 的 `ota` 字段轮询）：`idle / checking / update_available / downloading / rebooting / failed`；检查与下载都在 32KB 栈的专属线程跑（TLS 握手栈开销大，不能在 httpd 线程），进行中拒绝重复触发。
- **检查**：`GET /api/ota/check` → `releases/latest` API（必须带 User-Agent，否则 GitHub 403）→ 从**API 响应字段**取版本：`target_commitish`（CI 发布滚动 Release 时写入的 github.sha）截 7 位短 SHA 与本机比对，**不下载任何资产**（manifest.json 等资产域名国内直连不稳，检查必须只依赖 api.github.com）；取 `wol-rs.bin` 资产的 API 端点 URL + `browser_download_url` 留给升级用（`dev` 构建视为总是落后）。
- **升级**：`POST /api/ota` → 按序两路下载（不做镜像加速）：① **API 资产端点**（`api.github.com/.../releases/assets/{id}` + `Accept: application/octet-stream` → 302 到 CDN 签名 URL，走的域名与版本检查相同，国内可达性最好）；② `browser_download_url`（github.com 直链）仅作后备——**github.com 的 TCP 连接在国内常被中间设备中止**（`esp-tls: delayed connect error`，ESP_ERR_HTTP_CONNECT，TLS/HTTP 之前），api.github.com 却一直通，"检查成功、下载失败"就是这个域名差异，不是代码问题。每路失败递增退避重试 2 次（2/4s；esp-tls 只用 getaddrinfo 首个结果且 lwIP 有 DNS 缓存，拉开间隔让 TTL 过期重新解析）；`EspHttpConnection` 证书 bundle + **手动跟随 302**（esp_http_client 自动重定向跨主机有 "delayed connect error" 已知 bug，`http_get` 每跳新连接跟 Location）→ `EspOta::initiate_update_with_known_size`（按 Content-Length 只擦需用扇区）→ `update.write()` 分片写 → `complete()` 校验镜像并切换启动分区 → 2s 后 `esp_restart()`。两个坑：① TX 缓冲 `buffer_size_tx` 必须 ≥ 签名重定向 URL 的请求行长度（SAS+JWT 查询串 ~1KB，默认 512 会报 `HTTP_CLIENT: Out of buffer`）；② 下载请求的 `Accept` 必须是 `application/octet-stream`，否则 API 资产端点只回 JSON 元数据不 302。
- **分区表**：`partitions.csv` 为 OTA 双槽布局（otadata + ota_0/ota_1 各 0x3D0000）；**NVS 偏移（0x9000）与旧 factory 布局一致**，存量配置不受影响。老固件（factory 布局）无法通过 OTA 迁移，**首次需串口全量烧录新分区表**。
- 无 rollback（未开 `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE`）：`esp_ota_end` 校验失败会中止，不会切换启动分区。



- **一模块一文件**：每个功能放在 `src/<name>.rs`，模块顶部用 `//!` 写明对应原 C 组件。
- 日志统一用 `log` crate（`log::info!` / `warn!` / `error!`）；注意 `esp_idf_sys::*` glob 里也有 `log`，需要时写 `::log::info!`。
- 错误处理：内部函数返回 `Result`，最外层 `main` 对致命错误 `std::process::abort()`（等价 C 的 `ESP_ERROR_CHECK`）。
- 配置读写一律走 `config_store::get/save`；地址簿读写一律走 `mac_store::*`（内部加锁），不要直接读 NVS。
- JSON 统一用 `serde` 派生结构体 + `serde_json`（替代 cJSON / 手写 sb_add）；状态、配置、地址簿的视图集中在 `src/status.rs`，字段顺序与原 C 输出一致。

## 开发经验 / 注意事项

- **NVS blob 兼容**：`config_store`（260B）与 `mac_store`（341B）刻意保持与原 C `repr(C)` 结构逐字节一致；修改字段布局会使旧配置失效。
- **ffi / 宏不可见**：`TEMPERATURE_SENSOR_CONFIG_DEFAULT()`、`NETWORK_PROV_EVENT_HANDLER_*` 等含复合字面量的宏 bindgen 不可见，需手工构造结构体（见 `wifi_prov.rs` / `chip_temp.rs`）。`WIFI_INIT_CONFIG_DEFAULT()` 同样不可见，但其字段值**全部取自 esp-idf-sys 生成的 `WIFI_*` / `CONFIG_*` 常量**（与 esp-idf-svc 内部一致），改 menuconfig 会自动跟随，无需手工同步。
- **bindgen 命名**：枚举常量名通常去掉了类型 `_t` 后缀（如 `network_prov_security_NETWORK_PROV_SECURITY_1`）；`ssize_t` 绑定为 `isize`。以 `_t_` 开头的 bindgen 常量在 `match` 中触发 `non_upper_case_globals` 警告，可在函数上加 `#[allow(non_upper_case_globals)]`。
- **Wi-Fi 重连的接管时机**：`wifi_prov.rs` 的重连 handler（STA_DISCONNECTED → `esp_wifi_connect()`）只在「已配网直连」与 sim 分支注册；**首次 BLE 配网成功的路径**要在 `NETWORK_PROV_END` 回调里、`network_prov_mgr_deinit()` 之前查询 `is_wifi_provisioned` 并按需注册——管理器 deinit 会注销它自己的 WIFI_EVENT handler，没人接管的话断线后永不重连。不能在 `start()` 无条件注册：配网期间管理器靠"认证失败即放弃"收尾，我们的 handler 会在 auth fail 后无限重试与之打架。
- **板载 USB 的两个口（立创 ESP32S3R8N8：CH334F HUB + CH340K）**：① **CH340K ← UART0/GPIO43/44**——上面调试串口映射到这组脚后，**串口命令口（help/status/wifi-clear 菜单）**就从这个口进出（macOS 需 WCH 驱动，枚举为 `wchusbserial*`，用 `cu.*` 打开）；② **ESP32 原生 USB（GPIO19/20）= USB-Serial-JTAG**，即 `/dev/cu.usbmodem*`，次级控制台把 log 镜像到这里。两个下行口引到同一个 Type-C。
- **串口命令口的实现要点**（`debug_uart.rs` transport + `serial_cmd.rs` 命令层，两层分离）：esp-idf-hal `UartDriver::read` 把"超时无数据"映射为 `Err(ESP_ERR_TIMEOUT)`，**这是正常空闲态不是错误**（真错误才 sleep 兜底，否则读失败立即返回会让线程忙等烧满一核）；行缓冲上限 256 字节，超长进入丢弃态到换行才提示一次（逐字节提示会刷屏）；响应用 `embedded-io` 的 `write_all` + `flush`（= 等 TX 排空），该依赖因此**必须保留**（其它模块将来发串口也靠它）。加命令 = `handle()` 的 match 加分支 + `MENU` 加一行。
- **wifi-clear 的机制依据**：network_provisioning `manager.c:1352` 的 `network_prov_mgr_is_wifi_provisioned` 只看 STA 配置 SSID 是否非空；`network_prov_mgr_reset_wifi_provisioning`（`manager.c:2445`）内部就是 `esp_wifi_restore()`。所以"清 Wi-Fi 密码 + 重新配网" = `esp_wifi_restore()` 清 NVS → `esp_restart()` → 开机 `wifi_prov::start()` 检测到未配网自动进 BLE 配网。**不要**尝试运行时重新拉起配网：此时 mgr 已 `deinit`、配网结束时 BT/BLE 内存已释放（`network_prov_event_handler_free_btdm`），重启是最干净的路径。
- **MQTT 生命周期**：`control_mqtt.rs` 用 `EspMqttClient::new` 得到 `(client, connection)`：connection 交给专属事件线程用阻塞的 `connection.next()` 驱动事件循环；client 交由**专属 API 工作线程（`mqtt_api`）独占**，事件线程与 HTTP 线程只通过 `API_TX` 队列投递 SubscribeAll/Publish 操作。`stop()` 先关队列 → 工作线程退出并 drop client → 事件通道关闭 → join 两个线程。状态主题统一取自 `app_config`。**事件线程栈必须 ≥8192**：事件处理含 serde_json 与大 FFI 栈结构（`wifi_ap_record_t` 约 800B），4KB 会触发栈金丝雀整机遇测复位（表象为发现命令无应答）。**client id 每设备唯一**（`wakemanager-<设备id>`），固定 ID 会被 broker 互踢导致发现时好时坏。
- **MQTT API 锁死锁（espressif/mqtt 1.1.0，2026-10-11 Wokwi 模拟发现）**：mqtt 任务循环在 API 锁内分发事件（`MQTT_API_LOCK → run_event_loop → dispatch → svc 回调 share() 阻塞等事件线程处理完，锁全程不放`）。因此**事件处理路径上（含 Connected 分支）调用任何 client API（subscribe/publish）都会互相等待死锁**——表象为连接成功但订阅/上线日志缺失、设备在平台上看不到。修复：所有 client API 走 `mqtt_api` 工作线程。诱因是 `Cargo.toml` 的 `espressif/mqtt version = "1.*"` 浮动版本在重建时拉到 1.1.0（事件循环新架构）；旧组件（10-10 真机验证时的解析结果）无此问题。
- **Wokwi 模拟网络限制**：不支持 BLE（`sim` feature 跳过配网）；出站 TCP/DNS/UDP(NTP) 正常，但长连接约 25s 被 NAT 断开（`MQTT error: ESP_FAIL → Disconnected → 自动重连`循环），真机不受影响。
- **HTTP 服务器持有**：`EspHttpServer` 非 `Send`（内含裸指针），`control_http.rs` 在专属线程内创建并持有它（不跨线程移动），因此无需 `unsafe impl Send/Sync`；`start()` 通过 channel 等待线程回报启动结果。
- **等待 IP**：`wifi_prov::wait_connected()` 用 `Mutex<bool> + Condvar` 阻塞等待（事件回调里 `notify_all`），不再轮询。
- **MQTT 数据不是 NUL 结尾**：`EventPayload::Received` 的 `topic`/`data` 需按长度/UTF-8 处理，避免越界。
- **看门狗**：`watchdog::init()` 会 `reconfigure`（IDF 默认已 `CONFIG_ESP_TASK_WDT_INIT` 自动初始化）；等待配网期间不要提前把主任务加入看门狗，应在开始循环喂狗后再 `add_current_task()`。
- **SNTP**：现在走 esp-idf-svc 的 `EspSntp`（server 字符串生命周期由句柄托管，`main()` 保活）；时区仍用 `setenv("TZ", ...)` + `tzset()`（0.53 的 `SntpConf` 无 timezone 字段）。
- **分区表**：`partitions.csv`（OTA 双槽布局）通过 `CONFIG_PARTITION_TABLE_CUSTOM=y` 编进构建；esp-idf-sys 的限制是**相对路径被解析到 cargo OUT_DIR**，`CUSTOM_FILENAME` 必须是绝对路径——写成 CMake 的 `"$ENV{PROJECT_DIR}/partitions.csv"`（esp-idf-sys 构建时始终把环境变量 `PROJECT_DIR` 设为 Cargo 工作区根目录，换机器不用改）。**不要用 `${PROJECT_PATH}`**：它只在 `project.cmake` 的 `__project_info()` 里有定义，而生成 `PARTITION_CSV_PATH` 的 `build.cmake` include sdkconfig.cmake 处变量为空，会退化成 `/partitions.csv`。flash size 相应设为 8MB。
  另注：esp-idf-sys 会把 `target/<triple>/<profile>/build/esp-idf-sys-*/out/sdkconfig` 作为**最后一个** defaults 文件传入（优先级最高），改 sdkconfig.defaults 时若该文件里已有旧值，需一并改掉或删除，否则不生效。
- **Windows 构建目录**：esp-idf-sys 要求 canonicalize 后的 OUT_DIR ≤ 88 字符；`target` 是指向短路径（`D:\espbuild`）的 NTFS junction——产物在项目内可见、检查时解析回短路径。换机器需重建 junction（`New-Item -ItemType Junction`），不能用 subst（canonicalize 会还原成原长路径）。
- **NVS 全新 flash 首启**：只读 `EspNvs::new(..., false)` 在命名空间不存在时返回 `ESP_ERR_NVS_NOT_FOUND`，`config_store`/`mac_store` 的 `init` 必须按"无存储配置"处理而非上抛（此 bug 由 Wokwi 模拟发现——真机全擦后首启会 abort 重启循环）。
- **Wokwi 模拟**：不支持 BLE → `sim` feature 跳过配网直连 `Wokwi-GUEST`；`wokwi/wokwi.toml` 不要设 `gdbServerPort`（会暂停仿真等 GDB）；esp-idf-sys 的 CMake 产物 `libespidf.bin` 不含 Rust 代码，模拟镜像必须用 cargo elf 经 `esptool elf2image` 转换（`wokwi/mkwokwi.sh`）；Wokwi 相关文件（`wokwi.toml`/`diagram.json`/`mkwokwi.sh`/产物 `build/`）统一在 `wokwi/` 子目录，Wokwi 扩展按 `**/wokwi.toml` 递归识别。
- **烧录 + 监视的正确姿势**：`cargo build` → `cargo espflash flash --monitor`（或 `cargo run`，runner 已配好）；**插件按钮也已打通**——`.vscode/settings.json` 配了 `idf.preFlashTask: ./wokwi/mkwokwi.sh --real`，点插件 Flash 会自动 cargo build（真机）+ 组装根 `build/`（`flasher_args.json`/`project_description.json`/`wol-rs.elf`/`wol-rs.bin`）再烧录，Monitor 用其中的 `project_description.json` 定位 elf、用根 sdkconfig 软链读波特率。`wokwi/build/` 是 sim 镜像，别拿去烧真机。**串口同时只能一个进程打开**：残留的 espflash/monitor 会话会让插件烧录、端口检测报 `Resource busy` / `Failed to detect the default serial port`（先 `lsof /dev/cu.usbmodem*` 排查）。
- **OTA 双槽的烧录陷阱**：app 永远烧进 ota_0（0x20000），但**启动哪个槽由 otadata**（0xf000/0x2000）决定——OTA 升级过的设备 otadata 指向另一槽（如 ota_1），只烧 ota_0 重启后跑的还是旧的下载版（"烧录后还是跑下载的版本"）。两条烧录链路都已重置启动槽：runner 带 `--erase-parts otadata`；`mkwokwi.sh` 生成的 `flasher_args.json` 带空白 `otadata.bin`（全 0xFF）写入 0xf000。手动 esptool 烧录记得先 `esptool.py --chip esp32s3 erase-region 0xf000 0x2000`（app 偏移是 0x20000，不是老 factory 布局的 0x10000）。
- **ESP-IDF 插件（VS Code）**：idf.py Build/menuconfig 对 Rust 工程不适用（无根 CMakeLists.txt）；**Monitor 可用**——工作区根的 `sdkconfig` 是指向 `target/.../esp-idf-sys-*/out/sdkconfig` 的软链（esp-idf-sys 升级后悬空需重建，见 README 命令）；**调试**用 `.vscode/launch.json` 的 "wol-rs: USB-JTAG 调试（ESP32-S3）"（`gdbtarget` + `runOpenOCD`，本机配置不入库），**`request` 必须是 `attach`**——`launch` 模式适配器会自己 spawn `gdbserver` 当被调试端（ESP32 没这个程序 → `spawn gdbserver ENOENT`），attach 才是连 OpenOCD 的 3333 端口、由 `runOpenOCD` 自动拉起 OpenOCD；调试前先 `pkill -f openocd` 清掉占 3333 端口的残留实例。
- **clangd 编译数据库**：`bindings.h` 里的 ESP-IDF 头文件（`esp_chip_info.h` 等）include 路径来自 esp-idf-sys 的 CMake 构建，clangd 无从得知 → 编辑器报 `'esp_chip_info.h' file not found`（**不是构建错误**，`cargo build` 不受影响）。`build.rs` 每次构建从最新 `esp-idf-sys-*/out/build/compile_commands.json` 提取全部 `-I`/`-D`（含托管组件），用真实编译命令探测工具链隐式目录（**必须用真实命令**：裸跑 `gcc -E -Wp,-v` 拿不到 picolibc 目录，`stdio.h` 会链断在 `__FILE`），补上两个托管组件守卫宏，为 `bindings.h` 合成一条条目写到 `target/clangd-db/compile_commands.json`（`.vscode/settings.json` 的 `--compile-commands-dir` 指向它）。不能放仓库根 `build/`——`mkwokwi.sh --real` 每次 `rm -rf` 它。`cargo clean` 后重新 `cargo build` 即恢复；VS Code 里 clangd 报旧缓存时执行 "clangd: Restart language server"。
- **日志时间戳**：`CONFIG_LOG_TIMESTAMP_SOURCE_SYSTEM=y`，串口日志显示 `HH:MM:SS.sss`（本地时间）；SNTP 校时前为 epoch。
- **`unsafe` 边界**：FFI 调用集中在各模块，回调用 `unsafe extern "C" fn`；`EspHttpServer` 与 MQTT 客户端都由各自专属线程持有，避免跨线程移动非 `Send` 类型。