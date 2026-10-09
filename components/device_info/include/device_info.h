/*
 * WakeManager - device identity helpers
 *
 * Exposes the device's unique identifier (derived from the Wi-Fi station MAC)
 * so it can be embedded in MQTT topics and status reports.
 */

#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/**
 * @brief Read the Wi-Fi station MAC address into @p mac (6 bytes).
 *
 * @return true on success, false if the MAC cannot be read.
 */
bool device_info_get_mac(uint8_t mac[6]);

/**
 * @brief Get the unique device identifier.
 *
 * The id is the 6-byte Wi-Fi station MAC formatted as 12 uppercase hex
 * characters without separators, e.g. "A1B2C3D4E5F6". Falls back to
 * "000000000000" if the MAC cannot be read.
 *
 * @param[out] out      Destination buffer
 * @param[in]  out_len  Size of @p out; should be at least 13 bytes.
 */
void device_info_get_id(char *out, size_t out_len);

#ifdef __cplusplus
}
#endif