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
 *   GET /                            - usage / status
 *   GET /wake                        - wake the default target
 *   GET /wake?mac=AA:BB:CC:DD:EE:FF  - wake the given target
 */
esp_err_t control_http_start(void);

/**
 * @brief Stop the HTTP control server.
 */
void control_http_stop(void);

#ifdef __cplusplus
}
#endif