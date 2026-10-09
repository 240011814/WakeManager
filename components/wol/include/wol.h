/*
 * WakeManager - Wake-on-LAN magic packet sender
 */

#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

/**
 * @brief Parse a MAC address string.
 *
 * Accepts "AA:BB:CC:DD:EE:FF", "AA-BB-CC-DD-EE-FF" and "AABBCCDDEEFF"
 * (case insensitive).
 *
 * @param[in]  str  Input string
 * @param[out] mac  Parsed 6-byte MAC
 * @return true on success, false if the string is not a valid MAC
 */
bool wol_parse_mac(const char *str, uint8_t mac[6]);

/**
 * @brief Format a 6-byte MAC as "AA:BB:CC:DD:EE:FF".
 */
void wol_mac_to_str(const uint8_t mac[6], char *out, size_t out_len);

/**
 * @brief Send a Wake-on-LAN magic packet for the given MAC.
 */
esp_err_t wol_send(const uint8_t mac[6]);

/**
 * @brief Parse a MAC string and send a magic packet for it.
 */
esp_err_t wol_send_str(const char *mac_str);

/**
 * @brief Send a magic packet to the configured default target.
 */
esp_err_t wol_send_default(void);

#ifdef __cplusplus
}
#endif