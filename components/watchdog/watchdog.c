/*
 * WakeManager - Task Watchdog (TWDT) control
 */

#include "watchdog.h"

#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#include "esp_task_wdt.h"
#include "esp_log.h"

#include "app_config.h"

static const char *TAG = "watchdog";

static bool s_ready = false;

esp_err_t watchdog_init(void)
{
    esp_task_wdt_config_t cfg = {
        .timeout_ms = WATCHDOG_TIMEOUT_MS,
        .idle_core_mask = (1U << portNUM_PROCESSORS) - 1,
        .trigger_panic = (WATCHDOG_TRIGGER_PANIC != 0),
    };

    /* The IDF may already have auto-initialized the TWDT
     * (CONFIG_ESP_TASK_WDT_INIT); in that case reconfigure it instead. */
    esp_err_t err = esp_task_wdt_reconfigure(&cfg);
    if (err == ESP_ERR_INVALID_STATE) {
        err = esp_task_wdt_init(&cfg);
    }
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "Failed to enable task watchdog: %s", esp_err_to_name(err));
        return err;
    }

    s_ready = true;
    ESP_LOGI(TAG, "Task watchdog enabled: timeout=%d ms, panic=%d",
             WATCHDOG_TIMEOUT_MS, WATCHDOG_TRIGGER_PANIC);
    return ESP_OK;
}

esp_err_t watchdog_add_current_task(void)
{
    if (!s_ready) {
        return ESP_ERR_INVALID_STATE;
    }
    esp_err_t err = esp_task_wdt_add(NULL);
    if (err != ESP_OK && err != ESP_ERR_INVALID_ARG) {
        ESP_LOGW(TAG, "Failed to subscribe task to watchdog: %s", esp_err_to_name(err));
    }
    return err;
}

void watchdog_feed(void)
{
    if (s_ready) {
        esp_task_wdt_reset();
    }
}