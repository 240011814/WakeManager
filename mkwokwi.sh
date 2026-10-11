#!/usr/bin/env bash
# 组装 Wokwi 模拟用的 build/ 目录（wokwi.toml 的 firmware 指向 build/flasher_args.json）：
#   bootloader/bootloader.bin      ← esp-idf-sys CMake 构建产物
#   partition_table/partition-table.bin  ← esp-idf-sys CMake 构建产物（自定义 3MB factory 表）
#   wol-rs.bin                     ← cargo 链接的固件 elf 经 esptool elf2-image 转换
#
# 用法：
#   ./mkwokwi.sh                  # 脚本内部先确保 cargo build --features sim
#   wokwi-cli .
# 注意：features 不同的构建共用同一个 elf 路径，脚本必须自己触发 sim 构建，
# 否则可能拿到上一次 default 构建的固件（BLE 配网路径，在 Wokwi 里会卡住）。
set -euo pipefail
cd "$(dirname "$0")"

TARGET_DIR="${CARGO_TARGET_DIR:-target}"        # 默认 <crate>/target（见 .cargo/config.toml 的 junction 说明）
PROFILE=debug
ELF="$TARGET_DIR/xtensa-esp32s3-espidf/$PROFILE/wol-rs"
IDF_PY="D:/esptools/python_env/idf6.1_py3.14_env/Scripts/python.exe"

cargo build --features sim 2>&1 | tail -1

# esp-idf-sys 的 CMake 构建目录（取最新一次构建）
SYS_BUILD=$(ls -td "$TARGET_DIR/xtensa-esp32s3-espidf/$PROFILE/build/esp-idf-sys-"*/out/build | head -1)
[ -f "$ELF" ] || { echo "firmware elf not found: $ELF (run 'cargo build --features sim' first)" >&2; exit 1; }
[ -f "$SYS_BUILD/bootloader/bootloader.bin" ] || { echo "esp-idf-sys build not found: $SYS_BUILD" >&2; exit 1; }

rm -rf build
mkdir -p build/bootloader build/partition_table
cp "$SYS_BUILD/bootloader/bootloader.bin" build/bootloader/
cp "$SYS_BUILD/partition_table/partition-table.bin" build/partition_table/

# flasher_args.json：Wokwi 据此装配完整 flash 镜像（app 指向固件镜像而非
# esp-idf-sys CMake 产物 libespidf.bin —— 那里面没有 Rust 代码）
cat > build/flasher_args.json <<'EOF'
{
    "write_flash_args" : [ "--flash-mode", "dio",
                           "--flash-size", "4MB",
                           "--flash-freq", "80m" ],
    "flash_settings" : {
        "flash_mode": "dio",
        "flash_size": "4MB",
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
"$IDF_PY" -m esptool --chip esp32s3 elf2image \
    --flash-mode dio --flash-freq 80m --flash-size 4MB \
    -o build/wol-rs.bin "$ELF"

ls -l build/bootloader/bootloader.bin build/partition_table/partition-table.bin build/wol-rs.bin
echo "OK: build/ ready for wokwi-cli"
