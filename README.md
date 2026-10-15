# WakeManager (Rust 版)

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
| `status` | `st` | 设备状态文本（与 `/api/status` 同一份数据）：id/ip/ssid/rssi/heap/mqtt/broadcast/macs/presence/temp/time |
| `csi` | — | CSI 近场感知实时量：state（calibrating/absent/present）/baseline/amp/std/drop/rssi/frames；`csi recal` 强制重新校准基线 |
| `wifi-clear` | `reset-wifi` | 清除 NVS 里的 Wi-Fi 配置（SSID/密码）→ 回复确认 → 300ms 后重启 → 开机检测到未配网，自动重新进入 BLE 配网（App 扫日志里的 QR / PoP）。**破坏性操作，设备需重新配网才能联网** |

示例（在串口终端里发 `help` / `status`）：

```
> help
==== WakeManager 串口命令 ====
help         显示本菜单
status       查看设备状态
csi          查看 CSI 近场感知状态（csi recal 重新校准基线）
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
    ├── serial_cmd.rs       # 串口命令分发（help / status / csi / wifi-clear）
    ├── wifi_prov.rs        # BLE 配网（network_provisioning FFI；sim feature 直连 Wokwi-GUEST）
    ├── index.html          # 内嵌单页 Web UI（原 components/wol_http/index.html）
    ├── control_http.rs     # EspHttpServer（index.html include_str 内嵌）
    ├── control_mqtt.rs     # EspMqttClient::new + 事件循环线程 + API 工作线程
    └── csi_presence.rs     # Wi-Fi CSI 近场人体感知（~1m 有人/无人，状态变化发 MQTT 通知）
```
