/*
 * WakeManager - Wi-Fi provisioning module
 *
 * Transport : Bluetooth LE (GATT)
 * Security  : protocomm Security 1 (X25519 + proof-of-possession + AES-CTR)
 *
 * In ESP-IDF v6.x the provisioning stack is provided by the registry component
 * `espressif/network_provisioning` (APIs prefixed `network_prov_*`).
 */

#pragma once

#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

/**
 * @brief Initialize Wi-Fi and the provisioning service.
 *
 * Brings up NVS, esp-netif, the default event loop, the Wi-Fi driver and the
 * provisioning manager. If the device is already provisioned it connects to
 * the stored AP; otherwise it starts the BLE provisioning service.
 *
 * @return ESP_OK on success, error code otherwise.
 */
esp_err_t wifi_prov_start(void);

/**
 * @brief Block until the device has obtained an IP address.
 */
void wifi_prov_wait_connected(void);

#ifdef __cplusplus
}
#endif