/*
 * WakeManager - runtime configuration store (NVS backed)
 */

#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

#define WM_CFG_MAC_LEN      18
#define WM_CFG_HOST_LEN     128
#define WM_CFG_USER_LEN     64
#define WM_CFG_PASS_LEN     64
#define WM_CFG_TOPIC_LEN    64
#define WM_CFG_BCAST_LEN    16

/* Runtime configuration. Compile-time defaults come from app_config.h and are
 * overridden by whatever is stored in NVS (set through the web UI). */
typedef struct {
    char     default_mac[WM_CFG_MAC_LEN];
    uint16_t wol_udp_port;
    char     wol_broadcast[WM_CFG_BCAST_LEN];
    bool     mqtt_enable;
    char     mqtt_uri[WM_CFG_HOST_LEN];
    char     mqtt_username[WM_CFG_USER_LEN];
    char     mqtt_password[WM_CFG_PASS_LEN];
    char     mqtt_cmd_topic[WM_CFG_TOPIC_LEN];
    char     mqtt_broadcast_topic[WM_CFG_TOPIC_LEN];
    char     mqtt_status_topic[WM_CFG_TOPIC_LEN];
} wm_config_t;

/**
 * @brief Load defaults from app_config.h into @p out.
 */
void config_store_defaults(wm_config_t *out);

/**
 * @brief Initialize the store. Requires NVS to be initialized already.
 */
esp_err_t config_store_init(void);

/**
 * @brief Copy the current configuration into @p out (thread safe).
 */
void config_store_get(wm_config_t *out);

/**
 * @brief Persist @p cfg to NVS and update the in-RAM copy.
 */
esp_err_t config_store_save(const wm_config_t *cfg);

#ifdef __cplusplus
}
#endif