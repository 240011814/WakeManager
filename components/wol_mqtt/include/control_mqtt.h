/*
 * WakeManager - MQTT control client
 */

#pragma once

#include <stdbool.h>

#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

/**
 * @brief Start the MQTT control client using the current stored config.
 *
 * Does nothing (returns ESP_OK) when MQTT is disabled in the config.
 */
esp_err_t control_mqtt_start(void);

/**
 * @brief Stop and destroy the MQTT client.
 */
esp_err_t control_mqtt_stop(void);

/**
 * @brief Stop then start again, picking up the latest stored config.
 */
esp_err_t control_mqtt_restart(void);

/**
 * @brief Whether the client is currently connected to the broker.
 */
bool control_mqtt_is_connected(void);

#ifdef __cplusplus
}
#endif