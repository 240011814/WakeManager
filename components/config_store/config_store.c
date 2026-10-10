/*
 * WakeManager - runtime configuration store (NVS backed)
 *
 * Compile-time defaults live in app_config.h. On boot we load a stored blob
 * from NVS if present, otherwise we fall back to the defaults. The web UI
 * writes new values here.
 */

#include "config_store.h"

#include <string.h>

#include "app_config.h"
#include "nvs.h"
#include "nvs_flash.h"
#include "esp_log.h"

#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"

static const char *TAG = "config";

#define NVS_NAMESPACE   "wm_cfg"
#define NVS_KEY         "cfg"

static wm_config_t s_cfg;
static SemaphoreHandle_t s_lock;

void config_store_defaults(wm_config_t *out)
{
    if (out == NULL) {
        return;
    }
    memset(out, 0, sizeof(*out));
    out->wol_udp_port = WOL_UDP_PORT;
    out->mqtt_enable = (WOL_MQTT_ENABLE != 0);
    strlcpy(out->mqtt_uri, WOL_MQTT_BROKER_URI, sizeof(out->mqtt_uri));
    strlcpy(out->mqtt_username, WOL_MQTT_USERNAME, sizeof(out->mqtt_username));
    strlcpy(out->mqtt_password, WOL_MQTT_PASSWORD, sizeof(out->mqtt_password));
}

esp_err_t config_store_init(void)
{
    if (s_lock == NULL) {
        s_lock = xSemaphoreCreateMutex();
    }

    config_store_defaults(&s_cfg);

    nvs_handle_t handle;
    esp_err_t err = nvs_open(NVS_NAMESPACE, NVS_READONLY, &handle);
    if (err != ESP_OK) {
        ESP_LOGI(TAG, "No stored config, using defaults");
        return ESP_OK;
    }

    wm_config_t stored;
    size_t len = sizeof(stored);
    err = nvs_get_blob(handle, NVS_KEY, &stored, &len);
    nvs_close(handle);

    if (err == ESP_OK && len == sizeof(stored)) {
        s_cfg = stored;
        ESP_LOGI(TAG, "Config loaded from NVS");
    } else {
        ESP_LOGI(TAG, "No stored config, using defaults");
    }
    return ESP_OK;
}

void config_store_get(wm_config_t *out)
{
    if (out == NULL) {
        return;
    }
    if (s_lock != NULL) {
        xSemaphoreTake(s_lock, portMAX_DELAY);
    }
    *out = s_cfg;
    if (s_lock != NULL) {
        xSemaphoreGive(s_lock);
    }
}

esp_err_t config_store_save(const wm_config_t *cfg)
{
    if (cfg == NULL) {
        return ESP_ERR_INVALID_ARG;
    }

    /* Make sure every string is null-terminated */
    wm_config_t c = *cfg;
    c.mqtt_uri[sizeof(c.mqtt_uri) - 1] = '\0';
    c.mqtt_username[sizeof(c.mqtt_username) - 1] = '\0';
    c.mqtt_password[sizeof(c.mqtt_password) - 1] = '\0';

    nvs_handle_t handle;
    esp_err_t err = nvs_open(NVS_NAMESPACE, NVS_READWRITE, &handle);
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "nvs_open failed: %s", esp_err_to_name(err));
        return err;
    }

    err = nvs_set_blob(handle, NVS_KEY, &c, sizeof(c));
    if (err == ESP_OK) {
        err = nvs_commit(handle);
    }
    nvs_close(handle);

    if (err != ESP_OK) {
        ESP_LOGE(TAG, "Failed to save config: %s", esp_err_to_name(err));
        return err;
    }

    if (s_lock != NULL) {
        xSemaphoreTake(s_lock, portMAX_DELAY);
    }
    s_cfg = c;
    if (s_lock != NULL) {
        xSemaphoreGive(s_lock);
    }

    ESP_LOGI(TAG, "Config saved");
    return ESP_OK;
}