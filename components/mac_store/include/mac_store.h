/*
 * WakeManager - MAC/IP address book (NVS backed)
 *
 * Stores up to WM_MAC_STORE_MAX (10) MAC + IP pairs. Entries are written from
 * the MQTT set topic and queried from the MQTT get topic; they are also shown
 * in the web UI. The whole table is persisted as a single NVS blob.
 */

#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "esp_err.h"

#include "config_store.h"   /* WM_CFG_MAC_LEN */

#ifdef __cplusplus
extern "C" {
#endif

/* Maximum number of stored MAC/IP entries. */
#define WM_MAC_STORE_MAX    10
/* String length for a dotted IPv4 address (including the NUL terminator). */
#define WM_MAC_IP_LEN       16

/* One address book entry. @c mac is normalised to "AA:BB:CC:DD:EE:FF"; @c ip
 * may be empty, in which case the default broadcast is used when waking. */
typedef struct {
    char mac[WM_CFG_MAC_LEN];
    char ip[WM_MAC_IP_LEN];
} wm_mac_entry_t;

/* The whole address book: a count plus a fixed-capacity array. */
typedef struct {
    uint8_t        count;
    wm_mac_entry_t entries[WM_MAC_STORE_MAX];
} wm_mac_table_t;

/**
 * @brief Load the stored address book from NVS. Requires NVS to be initialized.
 */
esp_err_t mac_store_init(void);

/**
 * @brief Number of stored entries (0..WM_MAC_STORE_MAX).
 */
int mac_store_count(void);

/**
 * @brief Copy the whole address book into @p out (thread safe).
 */
void mac_store_get(wm_mac_table_t *out);

/**
 * @brief Add a MAC/IP pair, or update the IP of an existing MAC.
 *
 * @param[in] mac  MAC address (any of the formats accepted by wol_parse_mac)
 * @param[in] ip   Optional dotted IPv4 (NULL/empty keeps the entry usable with
 *                 the default broadcast destination)
 * @return ESP_OK on success, ESP_ERR_INVALID_ARG on a bad MAC/IP, or
 *         ESP_ERR_NO_MEM when the table is full and the MAC is new.
 */
esp_err_t mac_store_set(const char *mac, const char *ip);

/**
 * @brief Remove the entry matching @p mac.
 *
 * @return ESP_OK on success, ESP_ERR_NOT_FOUND if there is no such entry.
 */
esp_err_t mac_store_remove(const char *mac);

/**
 * @brief Remove every entry.
 */
esp_err_t mac_store_clear(void);

/**
 * @brief Look up the IP stored for @p mac.
 *
 * @param[in]  mac      MAC to look up
 * @param[out] ip       Buffer for the stored IP (may receive an empty string)
 * @param[in]  ip_len   Size of @p ip
 * @return true if the MAC is present, false otherwise.
 */
bool mac_store_get_ip(const char *mac, char *ip, size_t ip_len);

#ifdef __cplusplus
}
#endif