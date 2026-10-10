/*
 * WakeManager - HTTP control server
 *
 * Serves a small single-page UI plus a REST API:
 *   GET  /              -> index.html
 *   GET  /api/status    -> JSON status
 *   GET  /api/config    -> JSON config
 *   POST /api/config    -> save config (x-www-form-urlencoded) + reload MQTT
 *   GET  /api/wake      -> send a magic packet (optional ?mac=)
 *   GET  /api/mac       -> MAC/IP address book (managed over MQTT)
 */

#include "control_http.h"

#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <stdarg.h>
#include <ctype.h>

#include "esp_http_server.h"
#include "esp_log.h"
#include "esp_wifi.h"
#include "esp_netif.h"
#include "esp_system.h"

#include "app_config.h"
#include "config_store.h"
#include "device_info.h"
#include "wol.h"
#include "control_mqtt.h"
#include "mac_store.h"
#include "chip_temp.h"
#include "time_sync.h"

static const char *TAG = "http";

static httpd_handle_t s_server = NULL;

/* Embedded web UI (see components/wol_http/index.html) */
extern const uint8_t index_html_start[] asm("_binary_index_html_start");
extern const uint8_t index_html_end[] asm("_binary_index_html_end");

/* Response buffer. The default httpd runs a single worker task, so a static
 * buffer is safe here. */
static char s_json[1280];

/* ---------------- small helpers ---------------- */

static int sb_add(char *buf, size_t size, int off, const char *fmt, ...)
{
    if (off < 0 || (size_t)off >= size) {
        return off;
    }
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(buf + off, size - off, fmt, ap);
    va_end(ap);
    return (n < 0) ? off : off + n;
}

/* Append a JSON-escaped string (including surrounding quotes) */
static int sb_json_str(char *buf, size_t size, int off, const char *s)
{
    if (off < (int)size) {
        buf[off++] = '"';
    }
    for (; s != NULL && *s != '\0'; s++) {
        unsigned char c = (unsigned char)*s;
        if (c == '"' || c == '\\') {
            if (off + 1 < (int)size) {
                buf[off++] = '\\';
                buf[off++] = (char)c;
            }
        } else if (c == '\n') {
            if (off + 1 < (int)size) { buf[off++] = '\\'; buf[off++] = 'n'; }
        } else if (c == '\r') {
            if (off + 1 < (int)size) { buf[off++] = '\\'; buf[off++] = 'r'; }
        } else if (c == '\t') {
            if (off + 1 < (int)size) { buf[off++] = '\\'; buf[off++] = 't'; }
        } else if (c >= 0x20) {
            if (off + 1 < (int)size) {
                buf[off++] = (char)c;
            }
        }
    }
    if (off < (int)size) {
        buf[off++] = '"';
    }
    if (off < (int)size) {
        buf[off] = '\0';
    }
    return off;
}

static void url_decode(char *dst, const char *src, size_t dst_size)
{
    if (dst_size == 0) {
        return;
    }
    size_t o = 0;
    for (; *src != '\0' && o + 1 < dst_size; src++) {
        if (*src == '%' && isxdigit((unsigned char)src[1]) && isxdigit((unsigned char)src[2])) {
            char hex[3] = { src[1], src[2], '\0' };
            dst[o++] = (char)strtol(hex, NULL, 16);
            src += 2;
        } else if (*src == '+') {
            dst[o++] = ' ';
        } else {
            dst[o++] = *src;
        }
    }
    dst[o] = '\0';
}

/* Read a form field from a urlencoded body, URL-decoded into out */
static bool form_get(const char *body, const char *key, char *out, size_t out_size)
{
    char raw[512];
    if (httpd_query_key_value(body, key, raw, sizeof(raw)) != ESP_OK) {
        return false;
    }
    url_decode(out, raw, out_size);
    return true;
}

static char *read_body(httpd_req_t *req)
{
    if (req->content_len <= 0 || req->content_len > 4096) {
        return NULL;
    }
    char *buf = malloc(req->content_len + 1);
    if (buf == NULL) {
        return NULL;
    }
    int received = 0;
    while (received < req->content_len) {
        int r = httpd_req_recv(req, buf + received, req->content_len - received);
        if (r <= 0) {
            free(buf);
            return NULL;
        }
        received += r;
    }
    buf[received] = '\0';
    return buf;
}

static esp_err_t send_json(httpd_req_t *req, const char *json)
{
    httpd_resp_set_type(req, "application/json");
    httpd_resp_set_hdr(req, "Cache-Control", "no-store");
    return httpd_resp_sendstr(req, json);
}

/* ---------------- handlers ---------------- */

static esp_err_t index_get_handler(httpd_req_t *req)
{
    httpd_resp_set_type(req, "text/html");
    return httpd_resp_send(req, (const char *)index_html_start,
                           index_html_end - index_html_start);
}

static esp_err_t status_get_handler(httpd_req_t *req)
{
    char ip[16] = "0.0.0.0";
    esp_netif_t *netif = esp_netif_get_handle_from_ifkey("WIFI_STA_DEF");
    if (netif != NULL) {
        esp_netif_ip_info_t ip_info;
        if (esp_netif_get_ip_info(netif, &ip_info) == ESP_OK) {
            esp_ip4addr_ntoa(&ip_info.ip, ip, sizeof(ip));
        }
    }

    char ssid[33] = "";
    int rssi = 0;
    wifi_ap_record_t ap;
    if (esp_wifi_sta_get_ap_info(&ap) == ESP_OK) {
        memcpy(ssid, ap.ssid, sizeof(ap.ssid));
        ssid[32] = '\0';
        rssi = ap.rssi;
    }

    wm_config_t cfg;
    config_store_get(&cfg);

    char bcast[16] = "";
    wol_get_default_broadcast(bcast, sizeof(bcast));

    char dev_id[16] = "";
    device_info_get_id(dev_id, sizeof(dev_id));

    int o = 0;
    o = sb_add(s_json, sizeof(s_json), o, "{");
    o = sb_add(s_json, sizeof(s_json), o, "\"id\":");
    o = sb_json_str(s_json, sizeof(s_json), o, dev_id);
    o = sb_add(s_json, sizeof(s_json), o, ",\"ip\":");
    o = sb_json_str(s_json, sizeof(s_json), o, ip);
    o = sb_add(s_json, sizeof(s_json), o, ",\"ssid\":");
    o = sb_json_str(s_json, sizeof(s_json), o, ssid);
    o = sb_add(s_json, sizeof(s_json), o, ",\"rssi\":%d", rssi);
    o = sb_add(s_json, sizeof(s_json), o, ",\"heap\":%u", (unsigned)esp_get_free_heap_size());
    o = sb_add(s_json, sizeof(s_json), o, ",\"mqtt_connected\":%s",
               control_mqtt_is_connected() ? "true" : "false");
    o = sb_add(s_json, sizeof(s_json), o, ",\"default_mac\":");
    o = sb_json_str(s_json, sizeof(s_json), o, cfg.default_mac);
    o = sb_add(s_json, sizeof(s_json), o, ",\"broadcast\":");
    o = sb_json_str(s_json, sizeof(s_json), o, bcast);
    o = sb_add(s_json, sizeof(s_json), o, ",\"mac_count\":%d", mac_store_count());

    float celsius;
    if (chip_temp_read(&celsius) == ESP_OK) {
        o = sb_add(s_json, sizeof(s_json), o, ",\"temp_c\":%.2f", (double)celsius);
    }
    if (time_sync_is_synced()) {
        char timebuf[32];
        time_sync_format(timebuf, sizeof(timebuf));
        o = sb_add(s_json, sizeof(s_json), o, ",\"time\":");
        o = sb_json_str(s_json, sizeof(s_json), o, timebuf);
    }
    o = sb_add(s_json, sizeof(s_json), o, "}");

    return send_json(req, s_json);
}

static esp_err_t config_get_handler(httpd_req_t *req)
{
    wm_config_t cfg;
    config_store_get(&cfg);

    int o = 0;
    o = sb_add(s_json, sizeof(s_json), o, "{");
    o = sb_add(s_json, sizeof(s_json), o, "\"default_mac\":");
    o = sb_json_str(s_json, sizeof(s_json), o, cfg.default_mac);
    o = sb_add(s_json, sizeof(s_json), o, ",\"wol_udp_port\":%u", cfg.wol_udp_port);
    o = sb_add(s_json, sizeof(s_json), o, ",\"wol_broadcast\":");
    o = sb_json_str(s_json, sizeof(s_json), o, cfg.wol_broadcast);
    o = sb_add(s_json, sizeof(s_json), o, ",\"mqtt_enable\":%s",
               cfg.mqtt_enable ? "true" : "false");
    o = sb_add(s_json, sizeof(s_json), o, ",\"mqtt_uri\":");
    o = sb_json_str(s_json, sizeof(s_json), o, cfg.mqtt_uri);
    o = sb_add(s_json, sizeof(s_json), o, ",\"mqtt_username\":");
    o = sb_json_str(s_json, sizeof(s_json), o, cfg.mqtt_username);
    o = sb_add(s_json, sizeof(s_json), o, ",\"mqtt_password\":");
    o = sb_json_str(s_json, sizeof(s_json), o, cfg.mqtt_password);
    o = sb_add(s_json, sizeof(s_json), o, "}");

    return send_json(req, s_json);
}

static esp_err_t config_post_handler(httpd_req_t *req)
{
    char *body = read_body(req);
    if (body == NULL) {
        httpd_resp_send_err(req, HTTPD_400_BAD_REQUEST, "Missing body");
        return ESP_FAIL;
    }

    wm_config_t cfg;
    config_store_get(&cfg);

    char tmp[WM_CFG_HOST_LEN];

    if (form_get(body, "default_mac", tmp, sizeof(tmp))) {
        strlcpy(cfg.default_mac, tmp, sizeof(cfg.default_mac));
    }
    if (form_get(body, "wol_udp_port", tmp, sizeof(tmp))) {
        long p = atol(tmp);
        if (p <= 0 || p > 65535) {
            free(body);
            httpd_resp_send_err(req, HTTPD_400_BAD_REQUEST, "Invalid UDP port");
            return ESP_FAIL;
        }
        cfg.wol_udp_port = (uint16_t)p;
    }
    if (form_get(body, "wol_broadcast", tmp, sizeof(tmp))) {
        strlcpy(cfg.wol_broadcast, tmp, sizeof(cfg.wol_broadcast));
    }
    if (form_get(body, "mqtt_enable", tmp, sizeof(tmp))) {
        cfg.mqtt_enable = (tmp[0] == '1' || tmp[0] == 't' || tmp[0] == 'T' ||
                           tmp[0] == 'y' || tmp[0] == 'Y');
    }
    if (form_get(body, "mqtt_uri", tmp, sizeof(tmp))) {
        strlcpy(cfg.mqtt_uri, tmp, sizeof(cfg.mqtt_uri));
    }
    if (form_get(body, "mqtt_username", tmp, sizeof(tmp))) {
        strlcpy(cfg.mqtt_username, tmp, sizeof(cfg.mqtt_username));
    }
    if (form_get(body, "mqtt_password", tmp, sizeof(tmp))) {
        strlcpy(cfg.mqtt_password, tmp, sizeof(cfg.mqtt_password));
    }
    free(body);

    /* Validate the default MAC if one is set */
    uint8_t mac[6];
    if (cfg.default_mac[0] != '\0' && !wol_parse_mac(cfg.default_mac, mac)) {
        httpd_resp_send_err(req, HTTPD_400_BAD_REQUEST, "Invalid default MAC");
        return ESP_FAIL;
    }

    if (config_store_save(&cfg) != ESP_OK) {
        httpd_resp_send_err(req, HTTPD_500_INTERNAL_SERVER_ERROR, "Failed to save config");
        return ESP_FAIL;
    }

    /* Apply MQTT changes immediately */
    control_mqtt_restart();

    return send_json(req, "{\"ok\":true}");
}

static esp_err_t wake_get_handler(httpd_req_t *req)
{
    char query[128] = {0};
    char raw[64];
    char mac_str[WM_CFG_MAC_LEN] = {0};
    char ip_str[16] = {0};

    if (httpd_req_get_url_query_str(req, query, sizeof(query)) == ESP_OK) {
        if (httpd_query_key_value(query, "mac", raw, sizeof(raw)) == ESP_OK) {
            url_decode(mac_str, raw, sizeof(mac_str));
        }
        if (httpd_query_key_value(query, "ip", raw, sizeof(raw)) == ESP_OK) {
            url_decode(ip_str, raw, sizeof(ip_str));
        }
    }

    esp_err_t err;
    if (mac_str[0] != '\0') {
        err = wol_send_str_to(mac_str, (ip_str[0] != '\0') ? ip_str : NULL);
    } else {
        err = wol_send_default();
    }

    if (err != ESP_OK) {
        httpd_resp_send_err(req, HTTPD_400_BAD_REQUEST, "Failed to send magic packet");
        return ESP_FAIL;
    }

    char resp[96];
    snprintf(resp, sizeof(resp), "{\"ok\":true,\"mac\":\"%s\",\"ip\":\"%s\"}",
             mac_str, ip_str);
    return send_json(req, resp);
}

/* GET /api/mac -> the stored MAC/IP address book (managed over MQTT). */
static esp_err_t mac_get_handler(httpd_req_t *req)
{
    wm_mac_table_t table;
    mac_store_get(&table);

    int o = 0;
    o = sb_add(s_json, sizeof(s_json), o, "{\"count\":%d,\"macs\":[", table.count);
    for (int i = 0; i < table.count; i++) {
        o = sb_add(s_json, sizeof(s_json), o, "%s{\"mac\":", i ? "," : "");
        o = sb_json_str(s_json, sizeof(s_json), o, table.entries[i].mac);
        o = sb_add(s_json, sizeof(s_json), o, ",\"ip\":");
        o = sb_json_str(s_json, sizeof(s_json), o, table.entries[i].ip);
        o = sb_add(s_json, sizeof(s_json), o, "}");
    }
    o = sb_add(s_json, sizeof(s_json), o, "]}");

    return send_json(req, s_json);
}

static const httpd_uri_t s_uris[] = {
    { .uri = "/",               .method = HTTP_GET,  .handler = index_get_handler },
    { .uri = "/api/status",     .method = HTTP_GET,  .handler = status_get_handler },
    { .uri = "/api/config",     .method = HTTP_GET,  .handler = config_get_handler },
    { .uri = "/api/config",     .method = HTTP_POST, .handler = config_post_handler },
    { .uri = "/api/wake",       .method = HTTP_GET,  .handler = wake_get_handler },
    { .uri = "/api/mac",        .method = HTTP_GET,  .handler = mac_get_handler },
};

esp_err_t control_http_start(void)
{
    httpd_config_t config = HTTPD_DEFAULT_CONFIG();
    config.server_port = WOL_HTTP_PORT;
    config.lru_purge_enable = true;
    config.max_uri_handlers = sizeof(s_uris) / sizeof(s_uris[0]);

    if (httpd_start(&s_server, &config) != ESP_OK) {
        ESP_LOGE(TAG, "Failed to start HTTP server on port %d", WOL_HTTP_PORT);
        s_server = NULL;
        return ESP_FAIL;
    }

    for (size_t i = 0; i < sizeof(s_uris) / sizeof(s_uris[0]); i++) {
        httpd_register_uri_handler(s_server, &s_uris[i]);
    }

    ESP_LOGI(TAG, "HTTP control server started on port %d", WOL_HTTP_PORT);
    return ESP_OK;
}

void control_http_stop(void)
{
    if (s_server != NULL) {
        httpd_stop(s_server);
        s_server = NULL;
    }
}