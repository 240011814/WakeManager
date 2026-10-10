/*
 * WakeManager - on-chip temperature sensor
 */

#include "chip_temp.h"

#include "driver/temperature_sensor.h"
#include "esp_log.h"

static const char *TAG = "chip_temp";

/* Measurement range: the internal sensor is most accurate in this window and
 * the driver rejects ranges it cannot cover. Values outside it clamp/fail, so
 * keep it comfortably around typical operating temperatures. */
#define CHIP_TEMP_RANGE_MIN     (-10)
#define CHIP_TEMP_RANGE_MAX     (80)

static temperature_sensor_handle_t s_sensor = NULL;

esp_err_t chip_temp_init(void)
{
    if (s_sensor != NULL) {
        return ESP_OK;
    }

    temperature_sensor_config_t cfg =
        TEMPERATURE_SENSOR_CONFIG_DEFAULT(CHIP_TEMP_RANGE_MIN, CHIP_TEMP_RANGE_MAX);

    esp_err_t err = temperature_sensor_install(&cfg, &s_sensor);
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "temperature_sensor_install failed: %s", esp_err_to_name(err));
        s_sensor = NULL;
        return err;
    }

    err = temperature_sensor_enable(s_sensor);
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "temperature_sensor_enable failed: %s", esp_err_to_name(err));
        temperature_sensor_uninstall(s_sensor);
        s_sensor = NULL;
        return err;
    }

    ESP_LOGI(TAG, "Internal temperature sensor enabled (range %d..%d C)",
             CHIP_TEMP_RANGE_MIN, CHIP_TEMP_RANGE_MAX);
    return ESP_OK;
}

bool chip_temp_is_ready(void)
{
    return s_sensor != NULL;
}

esp_err_t chip_temp_read(float *out_celsius)
{
    if (out_celsius == NULL) {
        return ESP_ERR_INVALID_ARG;
    }
    if (s_sensor == NULL) {
        return ESP_ERR_INVALID_STATE;
    }
    return temperature_sensor_get_celsius(s_sensor, out_celsius);
}