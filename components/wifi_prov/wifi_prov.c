/*
 * WakeManager - Wi-Fi provisioning module
 *
 * All Wi-Fi / provisioning configuration lives here. The application
 * (app_main.c) only calls wifi_prov_start() and wifi_prov_wait_connected().
 */

#include <stdio.h>
#include <string.h>

#include <freertos/FreeRTOS.h>
#include <freertos/event_groups.h>

#include <esp_log.h>
#include <esp_wifi.h>
#include <esp_event.h>
#include <esp_netif.h>

#include <network_provisioning/manager.h>
#include <network_provisioning/scheme_ble.h>

#include "wifi_prov.h"
#include "led_status.h"
#include "app_config.h"

static const char *TAG = "wifi_prov";

/* Signal a successful Wi-Fi connection on this event group */
#define WIFI_CONNECTED_EVENT    BIT0
static EventGroupHandle_t s_wifi_event_group;

static void event_handler(void *arg, esp_event_base_t event_base,
                          int32_t event_id, void *event_data)
{
    if (event_base == NETWORK_PROV_EVENT) {
        switch (event_id) {
        case NETWORK_PROV_START:
            ESP_LOGI(TAG, "Provisioning started");
            led_status_set(LED_STATUS_CONNECTING);
            break;
        case NETWORK_PROV_WIFI_CRED_RECV: {
            wifi_sta_config_t *wifi_sta_cfg = (wifi_sta_config_t *)event_data;
            ESP_LOGI(TAG, "Received Wi-Fi credentials\n\tSSID: %.*s",
                     (int)sizeof(wifi_sta_cfg->ssid),
                     (const char *)wifi_sta_cfg->ssid);
            break;
        }
        case NETWORK_PROV_WIFI_CRED_FAIL: {
            network_prov_wifi_sta_fail_reason_t *reason =
                (network_prov_wifi_sta_fail_reason_t *)event_data;
            ESP_LOGE(TAG, "Provisioning failed: %s",
                     (*reason == NETWORK_PROV_WIFI_STA_AUTH_ERROR) ?
                     "Wi-Fi station authentication failed" : "Wi-Fi access-point not found");
            led_status_set(LED_STATUS_CONNECTING);
            break;
        }
        case NETWORK_PROV_WIFI_CRED_SUCCESS:
            ESP_LOGI(TAG, "Provisioning successful");
            break;
        case NETWORK_PROV_END: {
            /* Provisioning service stopped; release the manager. */
            ESP_LOGI(TAG, "Provisioning ended");
            esp_err_t err = network_prov_mgr_deinit();
            if (err != ESP_OK) {
                ESP_LOGE(TAG, "Failed to de-initialize provisioning manager: %s",
                         esp_err_to_name(err));
            }
            break;
        }
        default:
            break;
        }
    } else if (event_base == WIFI_EVENT) {
        switch (event_id) {
        case WIFI_EVENT_STA_START:
            esp_wifi_connect();
            break;
        case WIFI_EVENT_STA_DISCONNECTED:
            ESP_LOGI(TAG, "Disconnected, reconnecting...");
            led_status_set(LED_STATUS_CONNECTING);
            esp_wifi_connect();
            break;
        default:
            break;
        }
    } else if (event_base == IP_EVENT && event_id == IP_EVENT_STA_GOT_IP) {
        ip_event_got_ip_t *event = (ip_event_got_ip_t *)event_data;
        ESP_LOGI(TAG, "Connected with IP address: " IPSTR, IP2STR(&event->ip_info.ip));
        led_status_set(LED_STATUS_CONNECTED);
        xEventGroupSetBits(s_wifi_event_group, WIFI_CONNECTED_EVENT);
    } else if (event_base == PROTOCOMM_TRANSPORT_BLE_EVENT) {
        switch (event_id) {
        case PROTOCOMM_TRANSPORT_BLE_CONNECTED:
            ESP_LOGI(TAG, "BLE transport: Connected");
            break;
        case PROTOCOMM_TRANSPORT_BLE_DISCONNECTED:
            ESP_LOGI(TAG, "BLE transport: Disconnected");
            break;
        default:
            break;
        }
    } else if (event_base == PROTOCOMM_SECURITY_SESSION_EVENT) {
        switch (event_id) {
        case PROTOCOMM_SECURITY_SESSION_SETUP_OK:
            ESP_LOGI(TAG, "Secured session established");
            break;
        case PROTOCOMM_SECURITY_SESSION_INVALID_SECURITY_PARAMS:
        case PROTOCOMM_SECURITY_SESSION_CREDENTIALS_MISMATCH:
            ESP_LOGE(TAG, "Failed to establish a secure session");
            break;
        default:
            break;
        }
    }
}

static void wifi_init_sta(void)
{
    ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_STA));
    ESP_ERROR_CHECK(esp_wifi_start());
}

static void get_device_service_name(char *service_name, size_t max)
{
    uint8_t eth_mac[6];
    esp_wifi_get_mac(WIFI_IF_STA, eth_mac);
    snprintf(service_name, max, "%s%02X%02X%02X",
             PROV_DEVICE_NAME_PREFIX, eth_mac[3], eth_mac[4], eth_mac[5]);
}

/* Optional application-specific endpoint. The payload format is chosen by
 * the application; plain ASCII is used here. The response buffer must be
 * heap-allocated, as it is freed by the protocomm layer after being sent. */
static esp_err_t custom_prov_data_handler(uint32_t session_id, const uint8_t *inbuf, ssize_t inlen,
                                          uint8_t **outbuf, ssize_t *outlen, void *priv_data)
{
    if (inbuf) {
        ESP_LOGI(TAG, "Received custom data: %.*s", (int)inlen, (const char *)inbuf);
    }
    char response[] = "SUCCESS";
    *outbuf = (uint8_t *)strdup(response);
    if (*outbuf == NULL) {
        ESP_LOGE(TAG, "System out of memory");
        return ESP_ERR_NO_MEM;
    }
    *outlen = strlen(response) + 1; /* +1 for the NUL terminator */
    return ESP_OK;
}

static void prov_print_qr(const char *name, const char *pop, const char *transport)
{
    if (!name || !transport) {
        ESP_LOGW(TAG, "Cannot generate provisioning payload: data missing");
        return;
    }
    char payload[150] = {0};
    if (pop) {
        snprintf(payload, sizeof(payload),
                 "{\"ver\":\"%s\",\"name\":\"%s\",\"pop\":\"%s\",\"transport\":\"%s\"}",
                 PROV_QR_VERSION, name, pop, transport);
    } else {
        snprintf(payload, sizeof(payload),
                 "{\"ver\":\"%s\",\"name\":\"%s\",\"transport\":\"%s\",\"network\":\"wifi\"}",
                 PROV_QR_VERSION, name, transport);
    }
    ESP_LOGI(TAG, "Scan this payload with the provisioning app (or generate a QR from it):");
    ESP_LOGI(TAG, "%s?data=%s", PROV_QRCODE_BASE_URL, payload);
}

esp_err_t wifi_prov_start(void)
{
    /* Note: NVS (and esp_netif/event loop) must be initialized by the caller.
     * app_main() calls nvs_flash_init() and config_store_init() first. */

    /* Initialize TCP/IP stack and the default event loop */
    ESP_ERROR_CHECK(esp_netif_init());
    ESP_ERROR_CHECK(esp_event_loop_create_default());
    s_wifi_event_group = xEventGroupCreate();

    /* Register provisioning, BLE transport, security and IP event handlers */
    ESP_ERROR_CHECK(esp_event_handler_register(NETWORK_PROV_EVENT, ESP_EVENT_ANY_ID, &event_handler, NULL));
    ESP_ERROR_CHECK(esp_event_handler_register(PROTOCOMM_TRANSPORT_BLE_EVENT, ESP_EVENT_ANY_ID, &event_handler, NULL));
    ESP_ERROR_CHECK(esp_event_handler_register(PROTOCOMM_SECURITY_SESSION_EVENT, ESP_EVENT_ANY_ID, &event_handler, NULL));
    ESP_ERROR_CHECK(esp_event_handler_register(IP_EVENT, IP_EVENT_STA_GOT_IP, &event_handler, NULL));

    /* Initialize Wi-Fi. The BLE transport runs the radio in STA mode. */
    esp_netif_create_default_wifi_sta();
    wifi_init_config_t wifi_cfg = WIFI_INIT_CONFIG_DEFAULT();
    ESP_ERROR_CHECK(esp_wifi_init(&wifi_cfg));

    /* Provisioning manager configuration: BLE GATT transport.
     * The scheme event handler frees the BT/BLE memory once provisioning is
     * done, since WakeManager does not need Bluetooth afterwards. */
    network_prov_mgr_config_t prov_cfg = {
        .scheme = network_prov_scheme_ble,
        .scheme_event_handler = NETWORK_PROV_SCHEME_BLE_EVENT_HANDLER_FREE_BTDM,
        .app_event_handler = NETWORK_PROV_EVENT_HANDLER_NONE,
    };
    ESP_ERROR_CHECK(network_prov_mgr_init(prov_cfg));

    bool provisioned = false;
    ESP_ERROR_CHECK(network_prov_mgr_is_wifi_provisioned(&provisioned));

    if (!provisioned) {
        ESP_LOGI(TAG, "Device is not provisioned, starting BLE provisioning");

        /* BLE device name / service name: <prefix><last 3 MAC bytes> */
        char service_name[32];
        get_device_service_name(service_name, sizeof(service_name));

        /* Security 1 (NETWORK_PROV_SECURITY_1):
         *   X25519 key exchange + proof of possession (pop) + AES-CTR.
         * The `pop` string must stay valid while provisioning is running. */
        const char *pop = PROV_POP;

        /* Optional application endpoint for custom configuration data. Must be
         * created before provisioning starts and registered after. */
        ESP_ERROR_CHECK(network_prov_mgr_endpoint_create("custom-data"));

        /* Start the BLE provisioning service */
        ESP_ERROR_CHECK(network_prov_mgr_start_provisioning(NETWORK_PROV_SECURITY_1,
                                                           (const void *)pop,
                                                           service_name, NULL));

        /* Register the handler for the optional endpoint created above */
        ESP_ERROR_CHECK(network_prov_mgr_endpoint_register("custom-data",
                                                           custom_prov_data_handler, NULL));

        prov_print_qr(service_name, pop, PROV_QR_TRANSPORT);
    } else {
        ESP_LOGI(TAG, "Device is already provisioned, starting Wi-Fi STA");

        /* No longer need the provisioning manager */
        ESP_ERROR_CHECK(network_prov_mgr_deinit());

        /* Enable reconnect handling and connect to the stored AP */
        ESP_ERROR_CHECK(esp_event_handler_register(WIFI_EVENT, ESP_EVENT_ANY_ID, &event_handler, NULL));
        wifi_init_sta();
    }

    return ESP_OK;
}

void wifi_prov_wait_connected(void)
{
    /* Wait until we have an IP address on the configured network */
    xEventGroupWaitBits(s_wifi_event_group, WIFI_CONNECTED_EVENT, pdTRUE, pdTRUE, portMAX_DELAY);
}