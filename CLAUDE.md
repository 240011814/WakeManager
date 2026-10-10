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
- **MQTT**：远程唤醒设备、发现设备（获取设备唯一 ID）。

核心能力：

- **网络唤醒**：向默认目标或指定 MAC 发送 WOL 魔术包；目标广播地址可自动推导。
- **运行时配置**：所有参数经 NVS 持久化，可在网页上修改。
- **设备唯一标识**：由 Wi-Fi STA MAC 生成 12 位十六进制 ID，用于 MQTT 寻址与发现。
- **设备发现**：所有设备订阅同一个广播主题，远程无需预先知道设备 ID 即可先发现再操作。
- **BLE 配网**：基于 `espressif/network_provisioning`。

技术栈：C（ESP-IDF / FreeRTOS），目标芯片 **ESP32-S3**。

## 常用命令

> 本机 ESP-IDF 未加入 PATH，需要先激活环境（脚本会把 `IDF_PATH` 指向 `~/.espressif/v6.1/esp-idf` 并切到对应 Python venv）：

```bash
# 激活 ESP-IDF 环境（每条新 shell 都需要）
. ~/.espressif/tools/activate_idf_v6.1.sh

# 构建
python "$IDF_PATH/tools/idf.py" build

# 烧录 + 串口监视
python "$IDF_PATH/tools/idf.py" -p /dev/ttyUSB0 flash monitor

# 仅构建单个组件（加快验证）
python "$IDF_PATH/tools/idf.py" build 2>&1 | tail -35

# 完整清理
python "$IDF_PATH/tools/idf.py" fullclean
```

生成物：`build/wol.bin`（应用镜像）。

## 项目结构

```
WakeManager/
├── CMakeLists.txt              # 工程入口（project(wol)，MINIMAL_BUILD）
├── sdkconfig / sdkconfig.defaults
├── partitions.csv
├── main/
│   └── app_main.c              # 入口：NVS/配置初始化 → 看门狗 → Wi-Fi 配网 → 校时 → 启动 HTTP/MQTT
└── components/                 # 每个功能模块一个组件（一模块一文件夹）
    ├── app_config/             # 编译期默认配置（纯宏，仅头文件；含固定 MQTT 主题）
    ├── config_store/           # 运行时配置：NVS 持久化 + 互斥锁保护
    ├── mac_store/              # MAC/IP 地址簿：最多 10 组，NVS 持久化 + 互斥锁保护
    ├── device_info/            # 设备唯一标识（Wi-Fi STA MAC → 12 位十六进制）
    ├── chip_temp/              # 芯片内部温度传感器（temperature_sensor 驱动）
    ├── time_sync/              # SNTP 自动校时（Wi-Fi 连接成功后启动）
    ├── watchdog/               # 任务看门狗（esp_task_wdt）
    ├── led_status/             # LED 状态指示（连接中/已连接等）
    ├── wifi_prov/              # Wi-Fi BLE 配网
    ├── wol/                    # 魔术包构造与发送（广播地址自动推导）
    ├── wol_http/               # HTTP 服务器 + 单页 Web UI（index.html 内嵌）
    └── wol_mqtt/               # MQTT 控制客户端（cJSON 解析命令）
```

托管组件（`managed_components/`，由 `idf_component.yml` 声明）：`espressif/mqtt`、`espressif/cjson`、`espressif/json_generator`、`espressif/network_provisioning`。

## 配置（config_store）

运行时配置以单个 blob 存于 NVS 命名空间 `wm_cfg`，字段定义见 `components/config_store/include/config_store.h` 的 `wm_config_t`；编译期默认值在 `components/app_config/include/app_config.h`。

| 字段 | 说明 |
| --- | --- |
| `default_mac` | 默认唤醒目标 MAC |
| `wol_udp_port` | 魔术包 UDP 端口（默认 9） |
| `wol_broadcast` | 广播地址；**留空或 `255.255.255.255` 时自动推导** |
| `mqtt_enable` | 是否启用 MQTT |
| `mqtt_uri` / `mqtt_username` / `mqtt_password` | Broker 与凭据 |

> **MQTT 主题不再是运行时可配置项**，全部固定为 `app_config.h` 中的宏：`WOL_MQTT_CMD_TOPIC`、`WOL_MQTT_BROADCAST_TOPIC`、`WOL_MQTT_STATUS_TOPIC`、`WOL_MQTT_MAC_TOPIC`、`WOL_MQTT_MAC_GET_TOPIC`、`WOL_MQTT_TEMP_TOPIC`。网页 UI 与 `/api/config` 已移除主题字段（升级后旧配置因 blob 长度变化会回退默认值）。

## MAC/IP 地址簿（mac_store）

`components/mac_store`：

- 最多存 **10 组** MAC + IP，整体以单个 blob 存于 NVS 命名空间 `wm_mac`（key `list`），互斥锁保护。
- MAC 统一规范化为 `AA:BB:CC:DD:EE:FF`；IP 可为空（唤醒时走默认广播）。
- API：`mac_store_init()`、`mac_store_count()`、`mac_store_get()`、`mac_store_set(mac, ip)`、`mac_store_remove(mac)`、`mac_store_clear()`、`mac_store_get_ip(mac, ...)`。
- 唤醒命令只给 `mac` 时，会自动用地址簿里该 MAC 存的 IP 作为目标（未命中则用默认广播）。

## 芯片温度（chip_temp）

`components/chip_temp`：封装 `esp_driver_tsens` 的内部温度传感器，`chip_temp_init()`（开机初始化一次）、`chip_temp_read(&celsius)`。

## 时间同步（time_sync）

`components/time_sync`：Wi-Fi 拿到 IP 后调用 `time_sync_start()`，通过 SNTP 校时；服务器与 POSIX 时区分别由 `NTP_SERVER`、`TIME_TIMEZONE`（默认 `pool.ntp.org` / `CST-8`）固定。`time_sync_is_synced()` 判断是否已同步，`time_sync_format()` 输出本地时间字符串。

## 看门狗（watchdog）

`components/watchdog`：`watchdog_init()` 配置并开启任务看门狗（`esp_task_wdt`，超时 `WATCHDOG_TIMEOUT_MS`，`WATCHDOG_TRIGGER_PANIC` 决定超时是否复位），监控各核 idle 任务；主任务在 Wi-Fi 连接后 `watchdog_add_current_task()` 订阅并在循环中 `watchdog_feed()`。中断看门狗由 Kconfig `CONFIG_ESP_INT_WDT`（默认开）控制，无需运行时配置。

## 网络唤醒（WOL）行为

`components/wol`：

- 魔术包 = 6 字节 `0xFF` + 目标 MAC 重复 16 次（共 102 字节），UDP 广播发送。
- **广播地址自动推导**：`directed broadcast = (ip & netmask) | ~netmask`，用网卡（`WIFI_STA_DEF`）的 IP 与掩码计算，支持任意前缀长度（如 `192.168.1.88/24 → 192.168.1.255`）。
  - 配置了明确地址（且非 `255.255.255.255`）→ 用配置值；
  - 否则自动推导；网卡无 IP 时回退 `255.255.255.255`。
- API：`wol_send(mac)`、`wol_send_to(mac, dest_ip)`、`wol_send_str*()`、`wol_get_default_broadcast()`。

## HTTP 接口

`components/wol_http`（默认端口 80）：

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/` | 内嵌单页 UI |
| GET | `/api/status` | 状态 JSON：`id`、`ip`、`ssid`、`rssi`、`heap`、`mqtt_connected`、`default_mac`、`broadcast`、`mac_count`、`temp_c`、`time` |
| GET | `/api/config` | 当前配置 JSON |
| POST | `/api/config` | 保存配置（`x-www-form-urlencoded`），随后重载 MQTT |
| GET | `/api/wake` | 发送魔术包，可选 `?mac=<MAC>&ip=<IP>` |
| GET | `/api/mac` | 地址簿 JSON：`{"count":n,"macs":[{"mac":...,"ip":...}]}`（增删改经 MQTT） |

## MQTT 协议

`components/wol_mqtt`。设备 ID（`id`）= Wi-Fi STA MAC 的 12 位大写十六进制，如 `A1B2C3D4E5F6`。

**订阅四个主题**（主题全部固定，`<id>` = 设备 ID）：

1. **广播主题**（共享，`WOL_MQTT_BROADCAST_TOPIC` = `wakemanager/broadcast`）
   - 所有设备都订阅；远程在不知道任何设备 ID 时用它做**设备发现**。
   - 收到 `{"cmd":"status"}`（或缺省 `cmd`）→ 立即把自身状态（含 `id`）发布到状态主题。
   - **不接受唤醒命令**。

2. **唤醒主题**（每设备，`WOL_MQTT_CMD_TOPIC` + `/<id>`，如 `wakemanager/cmd/A1B2C3D4E5F6`）
   - 唤醒命令：`{"cmd":"wake","mac":"AA:BB:CC:DD:EE:FF","ip":"192.168.1.255"}`
   - `mac`/`ip` 均可选：省略 `mac` 用默认目标，省略 `ip` 优先用地址簿中该 MAC 存的 IP、否则用默认广播地址。

3. **地址簿设置主题**（每设备，`WOL_MQTT_MAC_TOPIC` + `/<id>`，如 `wakemanager/mac/A1B2C3D4E5F6`）
   - 新增/更新：`{"cmd":"set","mac":"AA:BB:CC:DD:EE:FF","ip":"192.168.1.5"}`（`add`/`update` 同义）
   - 删除：`{"cmd":"del","mac":"AA:BB:CC:DD:EE:FF"}`（`delete`/`remove` 同义）
   - 清空：`{"cmd":"clear"}`
   - 结果发布到状态主题（`ok: mac stored` / `error: address book full` 等）。

4. **地址簿获取主题**（每设备，`WOL_MQTT_MAC_GET_TOPIC` + `/<id>`，如 `wakemanager/mac/get/A1B2C3D4E5F6`）
   - 收到 `{"cmd":"get"}`（或空消息）→ 把完整列表发布到状态主题。
   - 列表 JSON：`{"id":"A1B2C3D4E5F6","cmd":"mac_list","count":2,"macs":[{"mac":"AA:BB:...","ip":"192.168.1.5"},...]}`

**发布：** 命令结果、发现应答、地址簿列表发布到 `WOL_MQTT_STATUS_TOPIC`（`wakemanager/status`）；芯片温度发布到 `WOL_MQTT_TEMP_TOPIC` + `/<id>`（如 `wakemanager/temp/A1B2C3D4E5F6`），**每 60 秒一次**：`{"id":"A1B2C3D4E5F6","temp_c":41.25}`。

**状态上报 JSON 示例：**

```json
{"id":"A1B2C3D4E5F6","ip":"192.168.1.88","ssid":"MyWiFi","rssi":-52,
 "heap":123456,"default_mac":"AA:BB:CC:DD:EE:FF",
 "broadcast":"192.168.1.255","mqtt_connected":true,
 "mac_count":2,"temp_c":41.25,"time":"2026-10-10 20:30:00"}
```

**典型流程（远程不知道设备 ID）：**

```
1) 远程 publish  广播主题        {"cmd":"status"}
2) 各设备 publish 状态主题        {... "id":"A1B2C3D4E5F6" ...}   ← 取得设备 ID
3) 远程 publish  wakemanager/cmd/A1B2C3D4E5F6   {"cmd":"wake","mac":"..."}
4) 远程 publish  wakemanager/mac/A1B2C3D4E5F6   {"cmd":"set","mac":"...","ip":"..."}
5) 远程 publish  wakemanager/mac/get/A1B2C3D4E5F6  {"cmd":"get"}
```

**命令格式仅支持 JSON**（不再支持纯文本）。

## 代码规范

- **一模块一文件夹**：每个功能放在 `components/<name>/`，自带 `CMakeLists.txt`、`include/` 头文件、实现 `.c`。
- 头文件统一 `#pragma once`，并以 `extern "C"` 包裹，便于将来 C++ 调用。
- 每个源文件用 `static const char *TAG = "...";` 配合 `ESP_LOGI/W/E` 打日志。
- 新增对外 API 时同步更新头文件注释（Doxygen 风格）。
- 配置读写一律走 `config_store_get/save`；地址簿读写一律走 `mac_store_*`（均内部加锁），不要直接读 NVS。

## 开发经验 / 注意事项

- **IDF v6.1 移除了内置 cJSON**：需通过 `idf_component.yml` 声明 `espressif/cjson` 托管组件，`#include "cJSON.h"` 才可用。
- **托管组件依赖**：在组件的 `idf_component.yml` 中声明后会自动加入依赖，**无需**写进 `CMakeLists.txt` 的 `REQUIRES`；但直接 `#include` 的核心组件（如 `esp_netif`、`esp_wifi`）**必须**写入 `REQUIRES`。
- **组件名**：日志组件叫 `log`（不是 `esp_log`）；温度传感器驱动组件叫 `esp_driver_tsens`；任务看门狗头文件 `esp_task_wdt.h` 在 `esp_system`。
- **MINIMAL_BUILD = ON**：新组件必须被 `main` 依赖链引用才会参与构建。
- **MQTT 数据不是 NUL 结尾**：`event->topic` / `event->data` 需按长度处理，日志用 `%.*s` 打印，避免越界。
- **MQTT 主题固定**：主题是 `app_config.h` 的宏，不再进 `config_store`，也不在网页 UI 配置；每设备主题 = 基名 + `/<设备ID>`。
- **配置结构变更**：`wm_config_t` 是整体 blob，改字段会使旧配置失效并回退默认值，升级时需知悉（本次移除主题字段即属此类）。
- **看门狗**：`watchdog_init()` 会 `reconfigure`（IDF 默认已 `CONFIG_ESP_TASK_WDT_INIT` 自动初始化），监控各核 idle 任务，`WATCHDOG_TRIGGER_PANIC=1` 时超时复位。若任务可能长时间阻塞（如等待配网），**不要**提前把它加入看门狗，应在开始循环喂狗后再 `watchdog_add_current_task()`。
- **SNTP**：`esp_sntp.h` 由 `lwip` 组件提供，需 `CONFIG_LWIP_SNTP_*`（默认开启）；时区用 `setenv("TZ", ...)` + `tzset()` 设置。
- **广播地址**：仅在配置为空或 `255.255.255.255` 时自动推导；填具体地址即覆盖。
- **本机构建**：`idf.py` 不在 PATH，需用 `python "$IDF_PATH/tools/idf.py"` 调用。
```