/*
 * WakeManager - on-chip temperature sensor
 *
 * Thin wrapper around the ESP-IDF temperature sensor driver. The internal
 * sensor measures the chip (die) temperature, which is what the periodic MQTT
 * report publishes.
 */

#pragma once

#include <stdbool.h>

#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

/**
 * @brief Install and enable the internal temperature sensor.
 *
 * Safe to call once at startup; further calls return ESP_OK without doing
 * anything.
 */
esp_err_t chip_temp_init(void);

/**
 * @brief Whether the sensor is installed and enabled.
 */
bool chip_temp_is_ready(void);

/**
 * @brief Read the current chip temperature in degrees Celsius.
 *
 * @param[out] out_celsius  Measured temperature
 * @return ESP_OK on success, ESP_ERR_INVALID_STATE if not initialized.
 */
esp_err_t chip_temp_read(float *out_celsius);

#ifdef __cplusplus
}
#endif