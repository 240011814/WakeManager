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

/* MQTT activity indication: a short flash that overlays the normal state. */
#define LED_ACTIVITY_BLINK_MS    100   /* half period of the activity flash  */
#define LED_ACTIVITY_DURATION_MS 2000  /* total time the LED keeps flashing  */
/* ----------------------------------------------------------------------- */

static volatile led_status_t s_status = LED_STATUS_CONNECTING;

/* While the current tick count is before this deadline, the LED flashes to
 * signal MQTT activity; afterwards it falls back to s_status. */
static volatile TickType_t s_activity_until = 0;
static volatile bool s_activity = false;

static void led_task(void *arg)
{
    bool on = false;

    while (1) {
        TickType_t now = xTaskGetTickCount();
        bool activity = s_activity && (int32_t)(now - s_activity_until) < 0;
        if (s_activity && !activity) {
            s_activity = false;
        }

        if (activity) {
            /* Fast blink while MQTT traffic is flowing; the normal state
             * resumes automatically once the window expires. */
            on = !on;
            gpio_set_level(LED_GPIO, on ? LED_ACTIVE_LEVEL : LED_OFF_LEVEL);
            vTaskDelay(pdMS_TO_TICKS(LED_ACTIVITY_BLINK_MS));
        } else if (s_status == LED_STATUS_CONNECTED) {
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

void led_status_activity(void)
{
    s_activity_until = xTaskGetTickCount() + pdMS_TO_TICKS(LED_ACTIVITY_DURATION_MS);
    s_activity = true;
}