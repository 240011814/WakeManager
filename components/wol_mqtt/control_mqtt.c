/*
 * WakeManager - MQTT control client
 *
 * All topics are fixed at build time (see app_config.h) and the device id
 * (12 hex chars, e.g. A1B2C3D4E5F6) is appended where a per-device topic is
 * needed.
 *
 * Subscribed topics:
 *   - broadcast (shared, WOL_MQTT_BROADCAST_TOPIC): discovery. A remote that
 *     does not know any device id publishes {"cmd":"status"} (or an empty
 *     message) here and every device replies on the status topic with its id.
 *   - wake (WOL_MQTT_CMD_TOPIC + "/" + id): wake a target:
 *       {"cmd":"wake","mac":"AA:BB:CC:DD:EE:FF","ip":"192.168.1.255"}
 *   - mac set (WOL_MQTT_MAC_TOPIC + "/" + id): manage the MAC/IP address book:
 *       {"cmd":"set","mac":"AA:BB:CC:DD:EE:FF","ip":"192.168.1.5"}
 *       {"cmd":"del","mac":"AA:BB:CC:DD:EE:FF"}
 *       {"cmd":"clear"}
 *   - mac get (WOL_MQTT_MAC_GET_TOPIC + "/" + id): query the address book:
 *       {"cmd":"get"}   (an empty message also triggers a reply)
 *
 * Published topics:
 *   - status (WOL_MQTT_STATUS_TOPIC): command results, discovery replies and
 *     the address book listing.
 *   - temperature (WOL_MQTT_TEMP_TOPIC + "/" + id): the internal temperature,
 *     published once per minute.
 *
 * Connection parameters (broker, username, password, enabled) come from
 * config_store and can be changed from the web UI at runtime.
 */

#include "control_mqtt.h"

#include <string.h>
#include <strings.h>
#include <stdio.h>
#include <ctype.h>

#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#include "mqtt_client.h"
#include "esp_log.h"
#include "esp_system.h"
#include "esp_netif.h"
#include "esp_wifi.h"
#include "cJSON.h"

#include "lwip/netdb.h"
#include "lwip/sockets.h"
#include "arpa/inet.h"

#include "app_config.h"
#include "config_store.h"
#include "device_info.h"
#include "wol.h"
#include "mac_store.h"
#include "chip_temp.h"
#include "time_sync.h"

static const char *TAG = "mqtt";

/* Buffer big enough for any fixed topic plus "/" and the 12-char device id. */
#define MQTT_TOPIC_MAX      (WM_CFG_HOST_LEN)

static esp_mqtt_client_handle_t s_client = NULL;
static volatile bool s_connected = false;

/* Persistent copy of the config: the MQTT client keeps pointers into these
 * strings for its whole lifetime, so they must outlive the client. */
static wm_config_t s_cfg;

/* Fully resolved topics (fixed base + "/" + device id where applicable). */
static char s_bcast_topic[MQTT_TOPIC_MAX];
static char s_wake_topic[MQTT_TOPIC_MAX];
static char s_mac_topic[MQTT_TOPIC_MAX];
static char s_mac_get_topic[MQTT_TOPIC_MAX];
static char s_temp_topic[MQTT_TOPIC_MAX];

/* Accumulator for a command split across MQTT_EVENT_DATA chunks */
static char s_cmd[256];
static int s_cmd_len = 0;

static bool s_temp_task_started = false;

/* Build "<base>/<device_id>" into dst. @p base may be empty (no topic). */
static void build_topic(char *dst, size_t dst_len, const char *base, const char *dev_id)
{
    if (base == NULL || base[0] == '\0') {
        dst[0] = '\0';
        return;
    }
    snprintf(dst, dst_len, "%s/%s", base, dev_id);
}

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

/* Publish a raw string to the status topic. */
static void publish_status(const char *msg)
{
    if (s_client != NULL) {
        int msg_id = esp_mqtt_client_publish(s_client, WOL_MQTT_STATUS_TOPIC,
                                             msg, 0, 1, 0);
        ESP_LOGI(TAG, "Publishing to \"%s\": \"%s\" (msg_id=%d)",
                 WOL_MQTT_STATUS_TOPIC, msg, msg_id);
    }
}

/* Build and publish a JSON snapshot of this device's status. Includes the
 * unique device id so a controller can tell devices apart. */
static void publish_device_status(void)
{
    char dev_id[16] = {0};
    device_info_get_id(dev_id, sizeof(dev_id));

    wm_config_t cfg;
    config_store_get(&cfg);

    /* Current IPv4 address of the station interface */
    char ip[16] = "0.0.0.0";
    esp_netif_t *netif = esp_netif_get_handle_from_ifkey("WIFI_STA_DEF");
    if (netif != NULL) {
        esp_netif_ip_info_t ip_info;
        if (esp_netif_get_ip_info(netif, &ip_info) == ESP_OK) {
            esp_ip4addr_ntoa(&ip_info.ip, ip, sizeof(ip));
        }
    }

    /* Current Wi-Fi link information */
    char ssid[33] = "";
    int rssi = 0;
    wifi_ap_record_t ap;
    if (esp_wifi_sta_get_ap_info(&ap) == ESP_OK) {
        memcpy(ssid, ap.ssid, sizeof(ap.ssid));
        ssid[32] = '\0';
        rssi = ap.rssi;
    }

    char broadcast[16] = "";
    wol_get_default_broadcast(broadcast, sizeof(broadcast));

    cJSON *root = cJSON_CreateObject();
    if (root == NULL) {
        publish_status("error: out of memory");
        return;
    }
    cJSON_AddStringToObject(root, "id", dev_id);
    cJSON_AddStringToObject(root, "ip", ip);
    cJSON_AddStringToObject(root, "ssid", ssid);
    cJSON_AddNumberToObject(root, "rssi", rssi);
    cJSON_AddNumberToObject(root, "heap", (double)esp_get_free_heap_size());
    cJSON_AddStringToObject(root, "default_mac", cfg.default_mac);
    cJSON_AddStringToObject(root, "broadcast", broadcast);
    cJSON_AddBoolToObject(root, "mqtt_connected", s_connected);
    cJSON_AddNumberToObject(root, "mac_count", mac_store_count());

    float celsius;
    if (chip_temp_read(&celsius) == ESP_OK) {
        cJSON_AddNumberToObject(root, "temp_c", (double)celsius);
    }
    if (time_sync_is_synced()) {
        char timebuf[32];
        time_sync_format(timebuf, sizeof(timebuf));
        cJSON_AddStringToObject(root, "time", timebuf);
    }

    char *payload = cJSON_PrintUnformatted(root);
    cJSON_Delete(root);
    if (payload == NULL) {
        publish_status("error: out of memory");
        return;
    }
    publish_status(payload);
    cJSON_free(payload);
}

/* Publish the stored MAC/IP address book as JSON. */
static void publish_mac_list(void)
{
    char dev_id[16] = {0};
    device_info_get_id(dev_id, sizeof(dev_id));

    wm_mac_table_t table;
    mac_store_get(&table);

    cJSON *root = cJSON_CreateObject();
    if (root == NULL) {
        publish_status("error: out of memory");
        return;
    }
    cJSON_AddStringToObject(root, "id", dev_id);
    cJSON_AddStringToObject(root, "cmd", "mac_list");
    cJSON_AddNumberToObject(root, "count", table.count);

    cJSON *arr = cJSON_AddArrayToObject(root, "macs");
    for (int i = 0; arr != NULL && i < table.count; i++) {
        cJSON *item = cJSON_CreateObject();
        if (item == NULL) {
            break;
        }
        cJSON_AddStringToObject(item, "mac", table.entries[i].mac);
        cJSON_AddStringToObject(item, "ip", table.entries[i].ip);
        cJSON_AddItemToArray(arr, item);
    }

    char *payload = cJSON_PrintUnformatted(root);
    cJSON_Delete(root);
    if (payload == NULL) {
        publish_status("error: out of memory");
        return;
    }
    publish_status(payload);
    cJSON_free(payload);
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

/* Parse a JSON command object. Any of the output buffers may be NULL to skip
 * that field. Returns false on malformed JSON. */
static bool parse_command(const char *text,
                          char *cmd_name, size_t cmd_len,
                          char *mac_str, size_t mac_len,
                          char *ip_str, size_t ip_len)
{
    cJSON *root = cJSON_Parse(text);
    if (root == NULL) {
        return false;
    }

    const cJSON *jcmd = cJSON_GetObjectItemCaseSensitive(root, "cmd");
    const cJSON *jmac = cJSON_GetObjectItemCaseSensitive(root, "mac");
    const cJSON *jip = cJSON_GetObjectItemCaseSensitive(root, "ip");
    if (cmd_name != NULL && cJSON_IsString(jcmd) && jcmd->valuestring != NULL) {
        strlcpy(cmd_name, jcmd->valuestring, cmd_len);
    }
    if (mac_str != NULL && cJSON_IsString(jmac) && jmac->valuestring != NULL) {
        strlcpy(mac_str, jmac->valuestring, mac_len);
    }
    if (ip_str != NULL && cJSON_IsString(jip) && jip->valuestring != NULL) {
        strlcpy(ip_str, jip->valuestring, ip_len);
    }
    cJSON_Delete(root);
    return true;
}

/* Wake command, received on the per-device topic "<cmd>/<device_id>":
 *   {"cmd":"wake","mac":"AA:BB:CC:DD:EE:FF","ip":"192.168.1.255"} */
static void handle_wake_command(const char *cmd)
{
    char buf[256];
    snprintf(buf, sizeof(buf), "%s", cmd);
    char *text = trim(buf);

    if (*text == '\0') {
        return;
    }

    char cmd_name[16] = {0};
    char mac_str[WM_CFG_MAC_LEN] = {0};
    char ip_str[16] = {0};
    if (!parse_command(text, cmd_name, sizeof(cmd_name),
                       mac_str, sizeof(mac_str), ip_str, sizeof(ip_str))) {
        ESP_LOGW(TAG, "Invalid JSON wake command: \"%s\"", cmd);
        publish_status("error: invalid JSON command");
        return;
    }

    if (cmd_name[0] != '\0' && strcasecmp(cmd_name, "wake") != 0) {
        ESP_LOGW(TAG, "Command \"%s\" is not accepted on the wake topic", cmd_name);
        publish_status("error: unsupported command on wake topic");
        return;
    }

    const char *target_mac = (mac_str[0] != '\0') ? mac_str : NULL;
    const char *target_ip = (ip_str[0] != '\0') ? ip_str : NULL;

    /* An explicit IP combined with a MAC from the address book makes it easy
     * to wake a named host: {"cmd":"wake","mac":"..."} uses the stored IP when
     * the command itself has none. */
    char stored_ip[WM_MAC_IP_LEN] = {0};
    if (target_mac != NULL && target_ip == NULL &&
        mac_store_get_ip(target_mac, stored_ip, sizeof(stored_ip)) && stored_ip[0] != '\0') {
        target_ip = stored_ip;
        ESP_LOGI(TAG, "Using stored IP %s for %s", stored_ip, target_mac);
    }

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

/* Address book update, received on the per-device MAC set topic:
 *   {"cmd":"set","mac":"AA:BB:CC:DD:EE:FF","ip":"192.168.1.5"}
 *   {"cmd":"del","mac":"AA:BB:CC:DD:EE:FF"}
 *   {"cmd":"clear"} */
static void handle_mac_set_command(const char *cmd)
{
    char buf[256];
    snprintf(buf, sizeof(buf), "%s", cmd);
    char *text = trim(buf);

    if (*text == '\0') {
        publish_status("error: empty command on mac topic");
        return;
    }

    char cmd_name[16] = {0};
    char mac_str[WM_CFG_MAC_LEN] = {0};
    char ip_str[WM_MAC_IP_LEN] = {0};
    if (!parse_command(text, cmd_name, sizeof(cmd_name),
                       mac_str, sizeof(mac_str), ip_str, sizeof(ip_str))) {
        ESP_LOGW(TAG, "Invalid JSON mac command: \"%s\"", cmd);
        publish_status("error: invalid JSON command");
        return;
    }

    if (strcasecmp(cmd_name, "set") == 0 || strcasecmp(cmd_name, "add") == 0 ||
        strcasecmp(cmd_name, "update") == 0) {
        if (mac_str[0] == '\0') {
            publish_status("error: no MAC address in command");
            return;
        }
        esp_err_t err = mac_store_set(mac_str, ip_str);
        if (err == ESP_OK) {
            publish_status("ok: mac stored");
        } else if (err == ESP_ERR_NO_MEM) {
            publish_status("error: address book full");
        } else {
            publish_status("error: invalid MAC or IP");
        }
    } else if (strcasecmp(cmd_name, "del") == 0 ||
               strcasecmp(cmd_name, "delete") == 0 ||
               strcasecmp(cmd_name, "remove") == 0) {
        if (mac_str[0] == '\0') {
            publish_status("error: no MAC address in command");
            return;
        }
        esp_err_t err = mac_store_remove(mac_str);
        publish_status(err == ESP_OK ? "ok: mac removed" : "error: mac not found");
    } else if (strcasecmp(cmd_name, "clear") == 0) {
        esp_err_t err = mac_store_clear();
        publish_status(err == ESP_OK ? "ok: address book cleared"
                                     : "error: failed to clear address book");
    } else {
        ESP_LOGW(TAG, "Unsupported command \"%s\" on mac topic", cmd_name);
        publish_status("error: unsupported command on mac topic");
    }
}

/* Address book query, received on the per-device MAC get topic. Any payload
 * (including an empty one) triggers a listing on the status topic. */
static void handle_mac_get_command(const char *cmd)
{
    (void)cmd;
    ESP_LOGI(TAG, "Address book requested");
    publish_mac_list();
}

/* Discovery command, received on the shared broadcast topic:
 *   {"cmd":"status"}   (a missing "cmd" is also treated as a status request)
 * Responds by publishing this device's status (with its unique id) so a remote
 * that does not know any device id yet can discover it. Wake is NOT accepted
 * here. */
static void handle_broadcast_command(const char *cmd)
{
    char buf[256];
    snprintf(buf, sizeof(buf), "%s", cmd);
    char *text = trim(buf);

    /* An empty ping still triggers an announcement. */
    if (*text == '\0') {
        publish_device_status();
        return;
    }

    char cmd_name[16] = {0};
    if (!parse_command(text, cmd_name, sizeof(cmd_name), NULL, 0, NULL, 0)) {
        ESP_LOGW(TAG, "Invalid JSON broadcast command: \"%s\"", cmd);
        publish_status("error: invalid JSON command");
        return;
    }

    if (cmd_name[0] == '\0' || strcasecmp(cmd_name, "status") == 0) {
        ESP_LOGI(TAG, "Status requested on broadcast topic, announcing device status");
        publish_device_status();
    } else {
        ESP_LOGW(TAG, "Ignoring command \"%s\" on broadcast topic", cmd_name);
    }
}

/* Publish the chip temperature once per minute to "<temp>/<device_id>". */
static void temp_report_task(void *arg)
{
    (void)arg;
    char dev_id[16] = {0};
    device_info_get_id(dev_id, sizeof(dev_id));

    while (1) {
        vTaskDelay(pdMS_TO_TICKS(TEMP_REPORT_INTERVAL_MS));

        if (!s_connected || s_client == NULL || s_temp_topic[0] == '\0') {
            continue;
        }

        float celsius;
        if (chip_temp_read(&celsius) != ESP_OK) {
            ESP_LOGW(TAG, "Failed to read chip temperature");
            continue;
        }

        char payload[96];
        snprintf(payload, sizeof(payload), "{\"id\":\"%s\",\"temp_c\":%.2f}",
                 dev_id, celsius);
        int msg_id = esp_mqtt_client_publish(s_client, s_temp_topic, payload, 0, 0, 0);
        ESP_LOGI(TAG, "Temperature report: %s -> \"%s\" (msg_id=%d)",
                 s_temp_topic, payload, msg_id);
    }
}

static void mqtt_event_handler(void *handler_args, esp_event_base_t base,
                               int32_t event_id, void *event_data)
{
    esp_mqtt_event_handle_t event = (esp_mqtt_event_handle_t)event_data;
    esp_mqtt_client_handle_t client = event->client;

    switch ((esp_mqtt_event_id_t)event_id) {
    case MQTT_EVENT_CONNECTED: {
        s_connected = true;
        struct { const char *topic; const char *label; } subs[] = {
            { s_bcast_topic,    "broadcast"  },
            { s_wake_topic,     "wake"       },
            { s_mac_topic,      "mac"        },
            { s_mac_get_topic,  "mac get"    },
        };
        for (size_t i = 0; i < sizeof(subs) / sizeof(subs[0]); i++) {
            if (subs[i].topic[0] != '\0') {
                int msg_id = esp_mqtt_client_subscribe(client, subs[i].topic, 1);
                ESP_LOGI(TAG, "Subscribed to %s topic %s (msg_id=%d)",
                         subs[i].label, subs[i].topic, msg_id);
            }
        }
        publish_status("online");
        break;
    }

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

        int topic_len = event->topic_len;
        bool is_wake = (s_wake_topic[0] != '\0' &&
                        topic_len == (int)strlen(s_wake_topic) &&
                        strncmp(event->topic, s_wake_topic, topic_len) == 0);
        bool is_bcast = (s_bcast_topic[0] != '\0' &&
                         topic_len == (int)strlen(s_bcast_topic) &&
                         strncmp(event->topic, s_bcast_topic, topic_len) == 0);
        bool is_mac = (s_mac_topic[0] != '\0' &&
                       topic_len == (int)strlen(s_mac_topic) &&
                       strncmp(event->topic, s_mac_topic, topic_len) == 0);
        bool is_mac_get = (s_mac_get_topic[0] != '\0' &&
                           topic_len == (int)strlen(s_mac_get_topic) &&
                           strncmp(event->topic, s_mac_get_topic, topic_len) == 0);
        if (!is_wake && !is_bcast && !is_mac && !is_mac_get) {
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
            if (is_wake) {
                handle_wake_command(s_cmd);
            } else if (is_mac) {
                handle_mac_set_command(s_cmd);
            } else if (is_mac_get) {
                handle_mac_get_command(s_cmd);
            } else {
                handle_broadcast_command(s_cmd);
            }
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

    /* Resolve all fixed topics; per-device ones get "/<device id>" appended. */
    char dev_id[16] = {0};
    device_info_get_id(dev_id, sizeof(dev_id));
    strlcpy(s_bcast_topic, WOL_MQTT_BROADCAST_TOPIC, sizeof(s_bcast_topic));
    build_topic(s_wake_topic, sizeof(s_wake_topic), WOL_MQTT_CMD_TOPIC, dev_id);
    build_topic(s_mac_topic, sizeof(s_mac_topic), WOL_MQTT_MAC_TOPIC, dev_id);
    build_topic(s_mac_get_topic, sizeof(s_mac_get_topic), WOL_MQTT_MAC_GET_TOPIC, dev_id);
    build_topic(s_temp_topic, sizeof(s_temp_topic), WOL_MQTT_TEMP_TOPIC, dev_id);

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
    ESP_LOGI(TAG, "MQTT client starting, broker: %s, id: %s", s_cfg.mqtt_uri, dev_id);
    ESP_LOGI(TAG, "Topics: broadcast=%s wake=%s mac=%s mac_get=%s temp=%s status=%s",
             s_bcast_topic, s_wake_topic, s_mac_topic, s_mac_get_topic,
             s_temp_topic, WOL_MQTT_STATUS_TOPIC);

    /* The temperature reporter runs for the lifetime of the client. */
    if (!s_temp_task_started) {
        if (xTaskCreate(temp_report_task, "mqtt_temp", 3072, NULL, 4, NULL) == pdPASS) {
            s_temp_task_started = true;
        } else {
            ESP_LOGW(TAG, "Failed to start temperature report task");
        }
    }

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