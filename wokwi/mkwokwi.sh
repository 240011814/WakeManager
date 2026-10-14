#!/usr/bin/env bash
# 组装可烧录镜像目录：bootloader + 分区表 + 固件镜像 + flasher_args.json
#   bootloader/bootloader.bin      ← esp-idf-sys CMake 构建产物
#   partition_table/partition-table.bin  ← esp-idf-sys CMake 构建产物（自定义 3MB factory 表）
#   wol-rs.bin                     ← cargo 链接的固件 elf 经 esptool elf2image 转换
#
# 用法（仓库根或 wokwi/ 目录均可）：
#   ./wokwi/mkwokwi.sh                 # sim 固件（--features sim）→ wokwi/build/，供 wokwi-cli 消费
#   ./wokwi/mkwokwi.sh --real          # 真机固件（默认 features）→ 仓库根 build/，
#                                      #   供 ESP-IDF 插件的 Flash 按钮消费
#                                      #   （idf.buildPath 默认 "build"，只认 <build>/flasher_args.json）
#   ./wokwi/mkwokwi.sh --real --release # 真机 release 固件（CI 发布用）
# 注意：sim / real 两种构建共用同一个 elf 路径，脚本必须自己触发对应特征的构建，
# 否则会拿到上一次另一种特征的固件（sim 在真机上会直连 Wokwi-GUEST、卡在配网）。
set -euo pipefail

WOKWI_DIR="$(cd "$(dirname "$0")" && pwd)"     # <repo>/wokwi
REPO_ROOT="$(dirname "$WOKWI_DIR")"            # cargo 工程根
cd "$WOKWI_DIR"

PROFILE=debug
RELEASE_FLAG=""
FEATURES_ARG="--features sim"                  # 模拟：跳过 BLE 配网
OUT_DIR="$WOKWI_DIR/build"
for arg in "$@"; do
    case "$arg" in
        --real)
            FEATURES_ARG=""                    # 真机：默认 features
            OUT_DIR="$REPO_ROOT/build"
            ;;
        --release)
            PROFILE="release"
            RELEASE_FLAG="--release"
            ;;
        -h|--help)
            sed -n '2,12p' "$0"; exit 0
            ;;
        *)
            echo "unknown option: $arg (see --help)" >&2; exit 1
            ;;
    esac
done

TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
ELF="$TARGET_DIR/xtensa-esp32s3-espidf/$PROFILE/wol-rs"

# cargo 需在工程根执行（wokwi/ 是子目录，cargo 也能向上找到 Cargo.toml，但显式更稳）
#
# 插件任务环境（ESP-IDF Pre Flash 等）会注入 IDF_PATH=…/esp-idf：esp-idf-sys 一见
# IDF_PATH 就当成"自定义 esp-idf 仓库"，忽略 .cargo/config.toml 的 ESP_IDF_VERSION
# 然后直接 panic（exit 101，表象为插件 Flash 报 "Flash task failed"）。
# 显式清掉，保持与终端环境一致。
unset IDF_PATH

LOG="$REPO_ROOT/.mkwokwi.log"
{
    echo "=== $(date '+%F %T')  cmd=cargo build $RELEASE_FLAG $FEATURES_ARG  (cwd=$REPO_ROOT)"
    env | grep -E '^(IDF|ESP|CARGO|MCU)' | sort | sed 's/^/    env: /'
} >> "$LOG"
if ! (cd "$REPO_ROOT" && cargo build $RELEASE_FLAG $FEATURES_ARG) >> "$LOG" 2>&1; then
    echo "cargo build 失败！完整日志: $LOG" >&2
    tail -30 "$LOG" >&2
    exit 1
fi
tail -1 "$LOG"

# 插件 Monitor 读工作区根 sdkconfig（取波特率）：软链悬空/缺失时自动重建
# （esp-idf-sys 升级或换 target 后 hash 目录会变）
SDKCONFIG_SRC=$(ls -td "$TARGET_DIR/xtensa-esp32s3-espidf/$PROFILE/build/esp-idf-sys-"*/out/sdkconfig 2>/dev/null | head -1)
if [ -n "$SDKCONFIG_SRC" ] && [ ! -e "$REPO_ROOT/sdkconfig" ]; then
    ln -sfn "$SDKCONFIG_SRC" "$REPO_ROOT/sdkconfig"
    echo "修复 sdkconfig 软链 → $SDKCONFIG_SRC"
fi

# esp-idf-sys 的 CMake 构建目录（取最新一次构建）
SYS_BUILD=$(ls -td "$TARGET_DIR/xtensa-esp32s3-espidf/$PROFILE/build/esp-idf-sys-"*/out/build | head -1)
[ -f "$ELF" ] || { echo "firmware elf not found: $ELF (run 'cargo build' first)" >&2; exit 1; }
[ -f "$SYS_BUILD/bootloader/bootloader.bin" ] || { echo "esp-idf-sys build not found: $SYS_BUILD" >&2; exit 1; }

# ESP-IDF venv 里的 python（跑 esptool elf2image）：不硬编码路径（原先是
# Windows 的 D:/...python.exe，换到 macOS 后失效）。路径从 esp-idf-sys 写的
# out/esp-idf-build.json 里取 venv_python，跟随当前机器与 IDF 版本；
# 可用环境变量 IDF_PY 覆盖。
OUT_JSON="$(dirname "$SYS_BUILD")/esp-idf-build.json"
if [ -z "${IDF_PY:-}" ] && [ -f "$OUT_JSON" ]; then
    IDF_PY=$(OUT_JSON="$OUT_JSON" python3 -c \
        'import json,os; print(json.load(open(os.environ["OUT_JSON"]))["venv_python"])' 2>/dev/null || true)
fi
if [ -n "${IDF_PY:-}" ] && [ -f "$IDF_PY" ]; then
    ESPTOOL=("$IDF_PY" -m esptool)
else
    # 兜底：直接用 PATH 里的 python（需已安装 esptool）
    ESPTOOL=(python3 -m esptool)
fi

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR/bootloader" "$OUT_DIR/partition_table"
cp "$SYS_BUILD/bootloader/bootloader.bin" "$OUT_DIR/bootloader/"
cp "$SYS_BUILD/partition_table/partition-table.bin" "$OUT_DIR/partition_table/"

# flasher_args.json：Wokwi / ESP-IDF 插件据此装配完整 flash 镜像（app 指向固件
# 镜像而非 esp-idf-sys CMake 产物 libespidf.bin —— 那里面没有 Rust 代码）
cat > "$OUT_DIR/flasher_args.json" <<'EOF'
{
    "write_flash_args" : [ "--flash-mode", "dio",
                           "--flash-size", "8MB",
                           "--flash-freq", "80m" ],
    "flash_settings" : {
        "flash_mode": "dio",
        "flash_size": "8MB",
        "flash_freq": "80m"
    },
    "flash_files" : {
        "0x0" : "bootloader/bootloader.bin",
        "0x8000" : "partition_table/partition-table.bin",
        "0x10000" : "wol-rs.bin"
    },
    "bootloader" : { "offset" : "0x0", "file" : "bootloader/bootloader.bin", "encrypted" : "false" },
    "partition-table" : { "offset" : "0x8000", "file" : "partition_table/partition-table.bin", "encrypted" : "false" },
    "app" : { "offset" : "0x10000", "file" : "wol-rs.bin", "encrypted" : "false" },
    "extra_esptool_args" : {
        "after"  : "hard-reset",
        "before" : "default-reset",
        "stub"   : true,
        "chip"   : "esp32s3"
    }
}
EOF

# 固件 elf → ESP app image（bootloader 只认 0xE9 头的镜像；objcopy 裸 bin 不行）
"${ESPTOOL[@]}" --chip esp32s3 elf2image \
    --flash-mode dio --flash-freq 80m --flash-size 8MB \
    -o "$OUT_DIR/wol-rs.bin" "$ELF"

# ESP-IDF 插件（VS Code）的 Monitor/Size 按钮要求 <build>/project_description.json，
# 且 app_elf 指向的文件要真实存在于 build 目录（插件做 join(buildDir, app_elf)）。
# 拷一份 elf 进去并生成该清单，插件的 Flash/Monitor 就能直接消费这个目录。
cp "$ELF" "$OUT_DIR/wol-rs.elf"
if [ -f "$SYS_BUILD/bootloader/bootloader.elf" ]; then
    cp "$SYS_BUILD/bootloader/bootloader.elf" "$OUT_DIR/bootloader/"
fi
cat > "$OUT_DIR/project_description.json" <<EOF
{
  "project_name": "wol-rs",
  "project_version": "1.0.0",
  "project_path": "$REPO_ROOT",
  "build_dir": "$OUT_DIR",
  "config_file": "$REPO_ROOT/sdkconfig",
  "config_defaults": "$REPO_ROOT/sdkconfig.defaults",
  "bootloader_elf": "bootloader/bootloader.elf",
  "app_elf": "wol-rs.elf",
  "app_bin": "wol-rs.bin",
  "build_type": "$PROFILE",
  "version": "v6.1",
  "config_environment": {}
}
EOF

ls -l "$OUT_DIR/bootloader/bootloader.bin" "$OUT_DIR/partition_table/partition-table.bin" "$OUT_DIR/wol-rs.bin" "$OUT_DIR/project_description.json"
if [ -n "$FEATURES_ARG" ]; then
    echo "OK: $OUT_DIR/ ready for wokwi-cli"
else
    echo "OK: $OUT_DIR/ ready for ESP-IDF 插件 Flash（idf.buildPath 默认 build/）"
fi
