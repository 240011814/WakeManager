/*
 * WakeManager - MQTT control client
 *
 * Subscribes to the configured command topic and wakes a target on a JSON
 * command:
 *   {"mac":"AA:BB:CC:DD:EE:FF","ip":"192.168.1.255"}
 * Both fields are optional:
 *   - "mac" omitted -> use the configured default target
 *   - "ip"  omitted -> use the default destination (configured broadcast, or
 *                      the subnet broadcast derived from this device's IP)
 * Results are published to the configured status topic.
 *
 * All connection parameters (broker, username, password, topics, enabled)
 * come from config_store and can be changed from the web UI at runtime.
 */

#include "control_mqtt.h"

#include <string.h>
#include <stdio.h>
#include <ctype.h>

#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#include "mqtt_client.h"
#include "esp_log.h"
#include "cJSON.h"

#include "lwip/netdb.h"
#include "lwip/sockets.h"
#include "arpa/inet.h"

#include "app_config.h"
#include "config_store.h"
#include "wol.h"

static const char *TAG = "mqtt";

static esp_mqtt_client_handle_t s_client = NULL;
static volatile bool s_connected = false;

/* Persistent copy of the config: the MQTT client keeps pointers into these
 * strings for its whole lifetime, so they must outlive the client. */
static wm_config_t s_cfg;

/* Accumulator for a command split across MQTT_EVENT_DATA chunks */
static char s_cmd[192];
static int s_cmd_len = 0;

/* One-shot DNS diagnostic: log the broker host and the IP(s) the device
 * resolves it to. Distinguishes DNS problems (e.g. a hijacked/fake DNS
 * returning an unroutable address) from firewall/routing problems. */
static void dns_debug_task(void *arg)
{
    const char *uri = (const char *)arg;

    const char *p = strstr(uri, "://");
    p = (p != NULL) ? p + 3 : uri;

    char host[128] = {0};
    char port[8] = "1883";

    const char *colon = strchr(p, ':');
    const char *slash = strchr(p, '/');
    size_t hlen = colon ? (size_t)(colon - p)
                        : (slash ? (size_t)(slash - p) : strlen(p));
    if (hlen >= sizeof(host)) {
        hlen = sizeof(host) - 1;
    }
    memcpy(host, p, hlen);
    host[hlen] = '\0';

    if (colon != NULL) {
        const char *pe = (slash != NULL && slash > colon) ? slash : p + strlen(p);
        size_t plen = pe - (colon + 1);
        if (plen >= sizeof(port)) {
            plen = sizeof(port) - 1;
        }
        memcpy(port, colon + 1, plen);
        port[plen] = '\0';
    }

    struct addrinfo hints = { .ai_family = AF_INET, .ai_socktype = SOCK_STREAM };
    struct addrinfo *res = NULL;
    int rc = getaddrinfo(host, port, &hints, &res);
    if (rc != 0 || res == NULL) {
        ESP_LOGE(TAG, "DNS: cannot resolve %s: %d", host, rc);
    } else {
        for (struct addrinfo *ai = res; ai != NULL; ai = ai->ai_next) {
            char ip[16] = "?";
            struct sockaddr_in *sin = (struct sockaddr_in *)ai->ai_addr;
            inet_ntoa_r(sin->sin_addr, ip, sizeof(ip));
            ESP_LOGI(TAG, "DNS: %s:%s -> %s", host, port, ip);
        }
        freeaddrinfo(res);
    }
    vTaskDelete(NULL);
}

static void publish_status(const char *msg)
{
    if (s_client != NULL && s_cfg.mqtt_status_topic[0] != '\0') {
        int msg_id = esp_mqtt_client_publish(s_client, s_cfg.mqtt_status_topic,
                                             msg, 0, 1, 0);
        ESP_LOGI(TAG, "Publishing to \"%s\": \"%s\" (msg_id=%d)",
                 s_cfg.mqtt_status_topic, msg, msg_id);
    }
}

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
    char buf[256];
    snprintf(buf, sizeof(buf), "%s", cmd);
    char *text = trim(buf);

    if (*text == '\0') {
        return;
    }

    /* Only JSON is accepted, e.g.
     *   {"mac":"AA:BB:CC:DD:EE:FF","ip":"192.168.1.255"}
     * Both fields are optional (see the file header). */
    cJSON *root = cJSON_Parse(text);
    if (root == NULL) {
        ESP_LOGW(TAG, "Invalid JSON command: \"%s\"", cmd);
        publish_status("error: invalid JSON command");
        return;
    }

    char mac_str[WM_CFG_MAC_LEN] = {0};
    char ip_str[16] = {0};

    const cJSON *jmac = cJSON_GetObjectItemCaseSensitive(root, "mac");
    const cJSON *jip = cJSON_GetObjectItemCaseSensitive(root, "ip");
    if (cJSON_IsString(jmac) && jmac->valuestring != NULL) {
        strlcpy(mac_str, jmac->valuestring, sizeof(mac_str));
    }
    if (cJSON_IsString(jip) && jip->valuestring != NULL) {
        strlcpy(ip_str, jip->valuestring, sizeof(ip_str));
    }
    cJSON_Delete(root);

    const char *target_mac = (mac_str[0] != '\0') ? mac_str : NULL;
    const char *target_ip = (ip_str[0] != '\0') ? ip_str : NULL;

    if (target_mac == NULL) {
        /* No MAC supplied. With no IP either, wake the configured default
         * target; with only an IP, wake the default target on that IP. */
        if (target_ip == NULL) {
            esp_err_t err = wol_send_default();
            publish_status(err == ESP_OK ? "ok: magic packet sent"
                                         : "error: failed to send magic packet");
            return;
        }
        wm_config_t cfg;
        config_store_get(&cfg);
        if (cfg.default_mac[0] == '\0') {
            ESP_LOGW(TAG, "Command has no MAC address: \"%s\"", cmd);
            publish_status("error: no MAC address in command");
            return;
        }
        target_mac = cfg.default_mac;
    }

    ESP_LOGI(TAG, "Wake request: mac=%s ip=%s", target_mac,
             target_ip ? target_ip : "(default)");

    esp_err_t err = wol_send_str_to(target_mac, target_ip);
    if (err == ESP_OK) {
        publish_status("ok: magic packet sent");
    } else if (err == ESP_ERR_INVALID_ARG) {
        publish_status("error: invalid MAC or IP in command");
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
        s_connected = true;
        ESP_LOGI(TAG, "Connected to broker, subscribing to %s", s_cfg.mqtt_cmd_topic);
        if (s_cfg.mqtt_cmd_topic[0] != '\0') {
            int msg_id = esp_mqtt_client_subscribe(client, s_cfg.mqtt_cmd_topic, 1);
            ESP_LOGI(TAG, "Subscribe sent, msg_id=%d, topic=%s (qos=1)",
                     msg_id, s_cfg.mqtt_cmd_topic);
        }
        publish_status("online");
        break;

    case MQTT_EVENT_SUBSCRIBED:
        ESP_LOGI(TAG, "Subscribed, msg_id=%d", event->msg_id);
        break;

    case MQTT_EVENT_UNSUBSCRIBED:
        ESP_LOGI(TAG, "Unsubscribed, msg_id=%d", event->msg_id);
        break;

    case MQTT_EVENT_PUBLISHED:
        ESP_LOGD(TAG, "Published, msg_id=%d", event->msg_id);
        break;

    case MQTT_EVENT_DISCONNECTED:
        s_connected = false;
        ESP_LOGW(TAG, "Disconnected from broker");
        break;

    case MQTT_EVENT_DATA: {
        /* Log every message the broker delivers, whatever the topic. Note the
         * topic and payload are not NUL-terminated, so use the lengths. */
        ESP_LOGI(TAG, "Message received: topic=\"%.*s\" payload=\"%.*s\" "
                      "(%d/%d bytes)%s",
                 event->topic_len, event->topic,
                 event->data_len, event->data,
                 event->current_data_offset + event->data_len,
                 event->total_data_len,
                 (event->current_data_offset + event->data_len < event->total_data_len)
                     ? " [partial]" : "");

        int cmd_len = strlen(s_cfg.mqtt_cmd_topic);
        if (event->topic_len != cmd_len ||
            strncmp(event->topic, s_cfg.mqtt_cmd_topic, event->topic_len) != 0) {
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

        if (s_cmd_len >= event->total_data_len) {
            ESP_LOGI(TAG, "Command received: \"%s\"", s_cmd);
            handle_command(s_cmd);
            s_cmd_len = 0;
        }
        break;
    }

    case MQTT_EVENT_ERROR:
        s_connected = false;
        if (event->error_handle != NULL) {
            ESP_LOGE(TAG, "MQTT error: type=%d, connect_return_code=%d, "
                          "tls_last_esp_err=0x%x, stack_err=0x%x, sock_errno=%d",
                     event->error_handle->error_type,
                     event->error_handle->connect_return_code,
                     (unsigned)event->error_handle->esp_tls_last_esp_err,
                     (unsigned)event->error_handle->esp_tls_stack_err,
                     event->error_handle->esp_transport_sock_errno);
        } else {
            ESP_LOGE(TAG, "MQTT error (no error handle)");
        }
        break;

    default:
        break;
    }
}

esp_err_t control_mqtt_start(void)
{
    config_store_get(&s_cfg);

    if (!s_cfg.mqtt_enable) {
        ESP_LOGI(TAG, "MQTT disabled by configuration");
        return ESP_OK;
    }
    if (s_cfg.mqtt_uri[0] == '\0') {
        ESP_LOGW(TAG, "MQTT enabled but no broker URI set");
        return ESP_ERR_INVALID_STATE;
    }

    /* Diagnostic: log the IP the broker hostname resolves to */
    char *uri_copy = strdup(s_cfg.mqtt_uri);
    if (uri_copy != NULL) {
        if (xTaskCreate(dns_debug_task, "mqtt_dns", 3072, uri_copy, 4, NULL) != pdPASS) {
            free(uri_copy);
        }
    }

    const esp_mqtt_client_config_t cfg = {
        .broker = {
            .address.uri = s_cfg.mqtt_uri,
        },
        .credentials = {
            .client_id = WOL_MQTT_CLIENT_ID,
            .username = (s_cfg.mqtt_username[0] != '\0') ? s_cfg.mqtt_username : NULL,
            .authentication = {
                .password = (s_cfg.mqtt_password[0] != '\0') ? s_cfg.mqtt_password : NULL,
            },
        },
    };

    s_client = esp_mqtt_client_init(&cfg);
    if (s_client == NULL) {
        ESP_LOGE(TAG, "Failed to init MQTT client");
        return ESP_FAIL;
    }

    esp_mqtt_client_register_event(s_client, ESP_EVENT_ANY_ID, mqtt_event_handler, NULL);
    ESP_LOGI(TAG, "MQTT client starting, broker: %s", s_cfg.mqtt_uri);
    return esp_mqtt_client_start(s_client);
}

esp_err_t control_mqtt_stop(void)
{
    s_connected = false;
    if (s_client != NULL) {
        esp_mqtt_client_stop(s_client);
        esp_mqtt_client_destroy(s_client);
        s_client = NULL;
    }
    return ESP_OK;
}

esp_err_t control_mqtt_restart(void)
{
    control_mqtt_stop();
    return control_mqtt_start();
}

bool control_mqtt_is_connected(void)
{
    return s_connected;
}