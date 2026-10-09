/*
 * WakeManager - HTTP control server
 */

#pragma once

#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

/**
 * @brief Start the HTTP control server.
 *
 * Endpoints:
 *   GET  /              - web UI (index.html)
 *   GET  /api/status    - device/Wi-Fi/MQTT status as JSON
 *   GET  /api/config    - current configuration as JSON
 *   POST /api/config    - update configuration (x-www-form-urlencoded)
 *   GET  /api/wake[?mac=AA:BB:CC:DD:EE:FF] - send a magic packet
 */
esp_err_t control_http_start(void);

/**
 * @brief Stop the HTTP control server.
 */
void control_http_stop(void);

#ifdef __cplusplus
}
#endif