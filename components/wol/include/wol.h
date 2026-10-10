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
 * @brief Send a magic packet for the given MAC to an explicit destination.
 *
 * @param[in] mac      Target 6-byte MAC
 * @param[in] dest_ip  Destination IPv4 as a dotted string (e.g. a subnet
 *                     broadcast "192.168.1.255" or a unicast address). When
 *                     NULL or empty, the subnet broadcast derived automatically
 *                     from this device's IP and netmask is used.
 * @return ESP_OK on success, ESP_ERR_INVALID_ARG on a bad destination IP.
 */
esp_err_t wol_send_to(const uint8_t mac[6], const char *dest_ip);

/**
 * @brief Parse a MAC string and send a magic packet for it.
 */
esp_err_t wol_send_str(const char *mac_str);

/**
 * @brief Parse a MAC string and send a magic packet to an explicit destination.
 *
 * @param[in] mac_str  MAC address string
 * @param[in] dest_ip  Optional destination IPv4 (NULL/empty = auto-derived
 *                     subnet broadcast)
 */
esp_err_t wol_send_str_to(const char *mac_str, const char *dest_ip);

/**
 * @brief Get the default destination address as a dotted string.
 *
 * This is the address used when a command does not supply its own IP: the
 * subnet broadcast derived from this device's IP and netmask (e.g.
 * 192.168.1.88/24 -> 192.168.1.255), falling back to 255.255.255.255.
 *
 * @param[out] out      Buffer for the result
 * @param[in]  out_len  Size of @p out
 */
void wol_get_default_broadcast(char *out, size_t out_len);

#ifdef __cplusplus
}
#endif