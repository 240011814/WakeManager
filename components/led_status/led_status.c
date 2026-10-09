/*
 * WakeManager - status indicator LED
 *
 * A small background task owns the LED and reacts to the state set by
 * led_status_set(). This keeps the Wi-Fi / provisioning code free of any
 * timing details.
 */

#include "led_status.h"

#include <freertos/FreeRTOS.h>
#include <freertos/task.h>

#include <driver/gpio.h>
#include <esp_log.h>

static const char *TAG = "led_status";

/* --- Indicator configuration ------------------------------------------- */
#define LED_GPIO            GPIO_NUM_48
#define LED_ACTIVE_LEVEL    0     /* 0 = active-low (LED lights when pin is low) */
#define LED_OFF_LEVEL       (!LED_ACTIVE_LEVEL)
#define LED_BLINK_MS        500   /* half period of the "connecting" blink   */
/* ----------------------------------------------------------------------- */

static volatile led_status_t s_status = LED_STATUS_CONNECTING;

static void led_task(void *arg)
{
    bool on = false;

    while (1) {
        if (s_status == LED_STATUS_CONNECTED) {
            /* Solid on while connected */
            gpio_set_level(LED_GPIO, LED_ACTIVE_LEVEL);
            vTaskDelay(pdMS_TO_TICKS(100));
        } else {
            /* Blink while provisioning / (re)connecting */
            on = !on;
            gpio_set_level(LED_GPIO, on ? LED_ACTIVE_LEVEL : LED_OFF_LEVEL);
            vTaskDelay(pdMS_TO_TICKS(LED_BLINK_MS));
        }
    }
}

void led_status_init(void)
{
    gpio_config_t io_conf = {
        .pin_bit_mask = 1ULL << LED_GPIO,
        .mode = GPIO_MODE_OUTPUT,
        .pull_up_en = GPIO_PULLUP_DISABLE,
        .pull_down_en = GPIO_PULLDOWN_DISABLE,
        .intr_type = GPIO_INTR_DISABLE,
    };
    ESP_ERROR_CHECK(gpio_config(&io_conf));
    gpio_set_level(LED_GPIO, LED_OFF_LEVEL);

    xTaskCreate(led_task, "led_status", 2048, NULL, 5, NULL);
    ESP_LOGI(TAG, "Indicator LED initialized on GPIO%d", LED_GPIO);
}

void led_status_set(led_status_t status)
{
    s_status = status;
}