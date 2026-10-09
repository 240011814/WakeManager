/*
 * WakeManager - HTTP control server
 *
 * Simple REST interface on top of the Wake-on-LAN sender:
 *   GET /                            -> usage text
 *   GET /wake                        -> wake the default target
 *   GET /wake?mac=AA:BB:CC:DD:EE:FF  -> wake the given target
 */

#include "control_http.h"

#include <string.h>
#include <stdio.h>

#include "esp_http_server.h"
#include "esp_log.h"

#include "app_config.h"
#include "wol.h"

static const char *TAG = "http";

static httpd_handle_t s_server = NULL;

static esp_err_t root_get_handler(httpd_req_t *req)
{
    static const char resp[] =
        "WakeManager\n"
        "Usage:\n"
        "  GET /                            this help\n"
        "  GET /wake                        wake the default target\n"
        "  GET /wake?mac=AA:BB:CC:DD:EE:FF  wake the given target\n";
    httpd_resp_set_type(req, "text/plain");
    return httpd_resp_send(req, resp, HTTPD_RESP_USE_STRLEN);
}

static esp_err_t wake_get_handler(httpd_req_t *req)
{
    char query[64] = {0};
    char mac_str[24] = {0};

    if (httpd_req_get_url_query_str(req, query, sizeof(query)) == ESP_OK) {
        if (httpd_query_key_value(query, "mac", mac_str, sizeof(mac_str)) != ESP_OK) {
            mac_str[0] = '\0';
        }
    }

    const char *target = (mac_str[0] != '\0') ? mac_str : WOL_DEFAULT_MAC;

    uint8_t mac[6];
    if (!wol_parse_mac(target, mac)) {
        httpd_resp_send_err(req, HTTPD_400_BAD_REQUEST, "Invalid MAC address");
        return ESP_FAIL;
    }

    if (wol_send(mac) != ESP_OK) {
        httpd_resp_send_err(req, HTTPD_500_INTERNAL_SERVER_ERROR, "Failed to send magic packet");
        return ESP_FAIL;
    }

    char resp[64];
    snprintf(resp, sizeof(resp), "Magic packet sent to %s\n", target);
    httpd_resp_set_type(req, "text/plain");
    return httpd_resp_send(req, resp, HTTPD_RESP_USE_STRLEN);
}

static const httpd_uri_t s_root_uri = {
    .uri = "/",
    .method = HTTP_GET,
    .handler = root_get_handler,
};

static const httpd_uri_t s_wake_uri = {
    .uri = "/wake",
    .method = HTTP_GET,
    .handler = wake_get_handler,
};

esp_err_t control_http_start(void)
{
    httpd_config_t config = HTTPD_DEFAULT_CONFIG();
    config.server_port = WOL_HTTP_PORT;
    config.lru_purge_enable = true;

    if (httpd_start(&s_server, &config) != ESP_OK) {
        ESP_LOGE(TAG, "Failed to start HTTP server on port %d", WOL_HTTP_PORT);
        s_server = NULL;
        return ESP_FAIL;
    }

    httpd_register_uri_handler(s_server, &s_root_uri);
    httpd_register_uri_handler(s_server, &s_wake_uri);

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