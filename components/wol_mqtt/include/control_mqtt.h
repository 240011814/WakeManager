/*
 * WakeManager - MQTT control client
 */

#pragma once

#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

/**
 * @brief Start the MQTT control client.
 *
 * Connects to the broker from app_config.h and subscribes to
 * WOL_MQTT_CMD_TOPIC. Incoming commands trigger a magic packet; the result
 * is published to WOL_MQTT_STATUS_TOPIC.
 */
esp_err_t control_mqtt_start(void);

#ifdef __cplusplus
}
#endif