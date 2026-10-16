# WakeManager 固件下载

> 本分支由 GitHub Actions 自动维护，仅存放构建产物，请勿手动提交。

## 最新构建 (rust @ 0577ef4)

烧录：`esptool.py --chip esp32s3 write_flash 0x0 bootloader/bootloader.bin 0x8000 partition_table/partition-table.bin 0x20000 wol-rs.bin`

| 文件 | 下载 |
| --- | --- |
| `wol-rs.bin` | [下载](latest/wol-rs.bin) |
| `wol-rs.elf` | [下载](latest/wol-rs.elf) |
| `bootloader/bootloader.bin` | [下载](latest/bootloader/bootloader.bin) |
| `partition_table/partition-table.bin` | [下载](latest/partition_table/partition-table.bin) |
| `flasher_args.json` | [下载](latest/flasher_args.json) |
| `SHA256SUMS` | [下载](latest/SHA256SUMS) |

## 版本归档

| 版本 | 目录 | GitHub Release |
| --- | --- | --- |
| v1.0.1 | [releases/v1.0.1](releases/v1.0.1) | [Release 页面](../../releases/tag/v1.0.1) |
| v1.0.0 | [releases/v1.0.0](releases/v1.0.0) | [Release 页面](../../releases/tag/v1.0.0) |
