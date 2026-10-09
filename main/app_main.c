/*
 * WakeManager - application entry point
 *
 * Wi-Fi / provisioning configuration lives in wifi_prov.c.
 */

#include <stdio.h>
#include <inttypes.h>

#include <freertos/FreeRTOS.h>
#include <freertos/task.h>

#include <esp_log.h>
#include <esp_chip_info.h>
#include <esp_flash.h>
#include <esp_system.h>
#include "sdkconfig.h"

#include "wifi_prov.h"
#include "led_status.h"
#include "app_config.h"
#include "control_http.h"
#include "control_mqtt.h"

static const char *TAG = "WakeManager";

/* Print basic information about the chip and flash, as reported at boot. */
static void print_chip_info(void)
{
    esp_chip_info_t chip_info;
    uint32_t flash_size;

    esp_chip_info(&chip_info);
    printf("This is %s chip with %d CPU core(s), %s%s%s%s, ",
           CONFIG_IDF_TARGET,
           chip_info.cores,
           (chip_info.features & CHIP_FEATURE_WIFI_BGN) ? "WiFi/" : "",
           (chip_info.features & CHIP_FEATURE_BT) ? "BT" : "",
           (chip_info.features & CHIP_FEATURE_BLE) ? "BLE" : "",
           (chip_info.features & CHIP_FEATURE_IEEE802154) ? ", 802.15.4 (Zigbee/Thread)" : "");

    unsigned major_rev = chip_info.revision / 100;
    unsigned minor_rev = chip_info.revision % 100;
    printf("silicon revision v%d.%d, ", major_rev, minor_rev);

    if (esp_flash_get_size(NULL, &flash_size) != ESP_OK) {
        printf("Get flash size failed");
        return;
    }

    printf("%" PRIu32 "MB %s flash\n", flash_size / (uint32_t)(1024 * 1024),
           (chip_info.features & CHIP_FEATURE_EMB_FLASH) ? "embedded" : "external");

    printf("Minimum free heap size: %" PRIu32 " bytes\n", esp_get_minimum_free_heap_size());
}

void app_main(void)
{
    /* Basic chip / flash information */
    print_chip_info();

    /* Status indicator: starts blinking until Wi-Fi is connected */
    led_status_init();
    led_status_set(LED_STATUS_CONNECTING);

    /* Initialize Wi-Fi and provisioning (or connect if already provisioned) */
    ESP_ERROR_CHECK(wifi_prov_start());

    /* Block until the device has an IP address on the configured network */
    wifi_prov_wait_connected();

    ESP_LOGI(TAG, "Free heap after Wi-Fi connected: %" PRIu32 " bytes", esp_get_free_heap_size());

    /* Start the control interfaces that trigger Wake-on-LAN packets */
#if WOL_HTTP_ENABLE
    ESP_ERROR_CHECK(control_http_start());
#endif
#if WOL_MQTT_ENABLE
    if (control_mqtt_start() != ESP_OK) {
        ESP_LOGW(TAG, "MQTT control failed to start (continuing)");
    }
#endif

    ESP_LOGI(TAG, "WakeManager ready");

    /* Nothing else to do in the main task; work happens in the HTTP/MQTT
     * handlers and the LED task. */
    while (1) {
        vTaskDelay(pdMS_TO_TICKS(10000));
    }
}