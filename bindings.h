/*
 * 为 espressif/network_provisioning 托管组件生成 Rust 绑定（esp-idf-sys bindgen）。
 *
 * 守卫宏在组件被构建时由 esp-idf-sys 构建脚本定义（远端组件名中 "/" → "__"）。
 */
#if defined(ESP_IDF_COMP_ESPRESSIF__NETWORK_PROVISIONING_ENABLED)
#include "network_provisioning/manager.h"
#include "network_provisioning/scheme_ble.h"
#endif

/* Boot chip/flash info (main) */
#include <esp_chip_info.h>
#include <esp_flash.h>
