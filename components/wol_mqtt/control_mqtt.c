/*
 * WakeManager - MQTT control client
 *
 * Subscribes to WOL_MQTT_CMD_TOPIC and wakes a target on command:
 *   "wake"                     -> wake the default target
 *   "wake AA:BB:CC:DD:EE:FF"   -> wake the given target
 *   "AA:BB:CC:DD:EE:FF"        -> wake the given target
 * Results are published to WOL_MQTT_STATUS_TOPIC.
 */

#include "control_mqtt.h"

#include <string.h>
#include <strings.h>
#include <stdio.h>
#include <ctype.h>

#include "mqtt_client.h"
#include "esp_log.h"

#include "app_config.h"
#include "wol.h"

static const char *TAG = "mqtt";

static esp_mqtt_client_handle_t s_client = NULL;

/* Accumulator for a command split across MQTT_EVENT_DATA chunks */
static char s_cmd[64];
static int s_cmd_len = 0;

static void publish_status(const char *msg)
{
    if (s_client != NULL) {
        esp_mqtt_client_publish(s_client, WOL_MQTT_STATUS_TOPIC, msg, 0, 1, 0);
    }
}

/* Trim leading/trailing whitespace in place and return the start pointer */
static char *trim(char *s)
{
    while (*s != '\0' && isspace((unsigned char)*s)) {
        s++;
    }
    char *end = s + strlen(s);
    while (end > s && isspace((unsigned char)end[-1])) {
        *--end = '\0';
    }
    return s;
}

static void handle_command(const char *cmd)
{
    char buf[64];
    snprintf(buf, sizeof(buf), "%s", cmd);
    char *text = trim(buf);

    if (*text == '\0') {
        return;
    }

    const char *target;
    if (strncasecmp(text, "wake", 4) == 0 && (text[4] == '\0' || isspace((unsigned char)text[4]))) {
        char *arg = trim(text + 4);
        target = (*arg != '\0') ? arg : WOL_DEFAULT_MAC;
    } else {
        target = text; /* assume the payload is a MAC address */
    }

    uint8_t mac[6];
    if (!wol_parse_mac(target, mac)) {
        ESP_LOGW(TAG, "Ignoring invalid command: %s", text);
        publish_status("error: invalid MAC address");
        return;
    }

    if (wol_send(mac) == ESP_OK) {
        char mac_str[18];
        wol_mac_to_str(mac, mac_str, sizeof(mac_str));
        char status[48];
        snprintf(status, sizeof(status), "ok: magic packet sent to %s", mac_str);
        publish_status(status);
    } else {
        publish_status("error: failed to send magic packet");
    }
}

static void mqtt_event_handler(void *handler_args, esp_event_base_t base,
                               int32_t event_id, void *event_data)
{
    esp_mqtt_event_handle_t event = (esp_mqtt_event_handle_t)event_data;
    esp_mqtt_client_handle_t client = event->client;

    switch ((esp_mqtt_event_id_t)event_id) {
    case MQTT_EVENT_CONNECTED:
        ESP_LOGI(TAG, "Connected to broker, subscribing to %s", WOL_MQTT_CMD_TOPIC);
        esp_mqtt_client_subscribe(client, WOL_MQTT_CMD_TOPIC, 1);
        publish_status("online");
        break;

    case MQTT_EVENT_DISCONNECTED:
        ESP_LOGW(TAG, "Disconnected from broker");
        break;

    case MQTT_EVENT_DATA: {
        /* Only act on the command topic; ignore anything else */
        if (event->topic_len != (int)strlen(WOL_MQTT_CMD_TOPIC) ||
            strncmp(event->topic, WOL_MQTT_CMD_TOPIC, event->topic_len) != 0) {
            break;
        }

        int offset = event->current_data_offset;
        if (offset == 0) {
            s_cmd_len = 0;
        }
        int space = (int)sizeof(s_cmd) - 1 - s_cmd_len;
        int copy = event->data_len;
        if (copy > space) {
            copy = space;
        }
        if (copy > 0) {
            memcpy(s_cmd + s_cmd_len, event->data, copy);
            s_cmd_len += copy;
        }
        s_cmd[s_cmd_len] = '\0';

        /* Command complete once we have the whole payload */
        if (s_cmd_len >= event->total_data_len) {
            ESP_LOGI(TAG, "Command received: %s", s_cmd);
            handle_command(s_cmd);
            s_cmd_len = 0;
        }
        break;
    }

    case MQTT_EVENT_ERROR:
        ESP_LOGE(TAG, "MQTT error");
        break;

    default:
        break;
    }
}

esp_err_t control_mqtt_start(void)
{
    const esp_mqtt_client_config_t cfg = {
        .broker = {
            .address.uri = WOL_MQTT_BROKER_URI,
        },
        .credentials = {
            .client_id = WOL_MQTT_CLIENT_ID,
        },
    };

    s_client = esp_mqtt_client_init(&cfg);
    if (s_client == NULL) {
        ESP_LOGE(TAG, "Failed to init MQTT client");
        return ESP_FAIL;
    }

    esp_mqtt_client_register_event(s_client, ESP_EVENT_ANY_ID, mqtt_event_handler, NULL);
    ESP_LOGI(TAG, "MQTT client starting, broker: %s", WOL_MQTT_BROKER_URI);
    return esp_mqtt_client_start(s_client);
}