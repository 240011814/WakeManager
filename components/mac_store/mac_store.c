/*
 * WakeManager - MAC/IP address book (NVS backed)
 *
 * The table is kept in a single NVS blob (namespace "wm_mac", key "list") and
 * guarded by a mutex so the MQTT, HTTP and main tasks can use it concurrently.
 */

#include "mac_store.h"

#include <string.h>
#include <strings.h>

#include "nvs.h"
#include "nvs_flash.h"
#include "esp_log.h"

#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"

#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>

#include "wol.h"

static const char *TAG = "mac_store";

#define NVS_NAMESPACE   "wm_mac"
#define NVS_KEY         "list"

static wm_mac_table_t s_table;
static SemaphoreHandle_t s_lock;

static void lock(void)
{
    if (s_lock != NULL) {
        xSemaphoreTake(s_lock, portMAX_DELAY);
    }
}

static void unlock(void)
{
    if (s_lock != NULL) {
        xSemaphoreGive(s_lock);
    }
}

/* Normalise a MAC string to "AA:BB:CC:DD:EE:FF". Returns false if invalid. */
static bool normalise_mac(const char *mac, char *out, size_t out_len)
{
    uint8_t bytes[6];
    if (!wol_parse_mac(mac, bytes)) {
        return false;
    }
    wol_mac_to_str(bytes, out, out_len);
    return true;
}

static bool ip_is_valid(const char *ip)
{
    if (ip == NULL || ip[0] == '\0') {
        return true;    /* empty is allowed: default broadcast is used */
    }
    struct in_addr addr;
    return inet_pton(AF_INET, ip, &addr) == 1;
}

static int find_index(const char *normalised_mac)
{
    for (int i = 0; i < s_table.count; i++) {
        if (strcasecmp(s_table.entries[i].mac, normalised_mac) == 0) {
            return i;
        }
    }
    return -1;
}

static esp_err_t persist(void)
{
    nvs_handle_t handle;
    esp_err_t err = nvs_open(NVS_NAMESPACE, NVS_READWRITE, &handle);
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "nvs_open failed: %s", esp_err_to_name(err));
        return err;
    }
    err = nvs_set_blob(handle, NVS_KEY, &s_table, sizeof(s_table));
    if (err == ESP_OK) {
        err = nvs_commit(handle);
    }
    nvs_close(handle);
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "Failed to save address book: %s", esp_err_to_name(err));
    }
    return err;
}

esp_err_t mac_store_init(void)
{
    if (s_lock == NULL) {
        s_lock = xSemaphoreCreateMutex();
    }

    memset(&s_table, 0, sizeof(s_table));

    nvs_handle_t handle;
    esp_err_t err = nvs_open(NVS_NAMESPACE, NVS_READONLY, &handle);
    if (err != ESP_OK) {
        ESP_LOGI(TAG, "No stored address book, starting empty");
        return ESP_OK;
    }

    wm_mac_table_t stored;
    size_t len = sizeof(stored);
    err = nvs_get_blob(handle, NVS_KEY, &stored, &len);
    nvs_close(handle);

    if (err == ESP_OK && len == sizeof(stored)) {
        if (stored.count > WM_MAC_STORE_MAX) {
            stored.count = WM_MAC_STORE_MAX;
        }
        s_table = stored;
        ESP_LOGI(TAG, "Address book loaded: %d entr%s", s_table.count,
                 s_table.count == 1 ? "y" : "ies");
    } else {
        ESP_LOGI(TAG, "No stored address book, starting empty");
    }
    return ESP_OK;
}

int mac_store_count(void)
{
    lock();
    int count = s_table.count;
    unlock();
    return count;
}

void mac_store_get(wm_mac_table_t *out)
{
    if (out == NULL) {
        return;
    }
    lock();
    *out = s_table;
    unlock();
}

bool mac_store_get_ip(const char *mac, char *ip, size_t ip_len)
{
    char norm[WM_CFG_MAC_LEN];
    if (ip == NULL || ip_len == 0 || !normalise_mac(mac, norm, sizeof(norm))) {
        return false;
    }

    bool found = false;
    lock();
    int idx = find_index(norm);
    if (idx >= 0) {
        strlcpy(ip, s_table.entries[idx].ip, ip_len);
        found = true;
    }
    unlock();
    return found;
}

esp_err_t mac_store_set(const char *mac, const char *ip)
{
    char norm[WM_CFG_MAC_LEN];
    if (!normalise_mac(mac, norm, sizeof(norm))) {
        ESP_LOGW(TAG, "Invalid MAC: %s", mac ? mac : "(null)");
        return ESP_ERR_INVALID_ARG;
    }
    if (!ip_is_valid(ip)) {
        ESP_LOGW(TAG, "Invalid IP: %s", ip);
        return ESP_ERR_INVALID_ARG;
    }
    const char *ip_str = (ip != NULL) ? ip : "";

    lock();
    int idx = find_index(norm);
    if (idx < 0) {
        if (s_table.count >= WM_MAC_STORE_MAX) {
            unlock();
            ESP_LOGW(TAG, "Address book full (%d entries)", WM_MAC_STORE_MAX);
            return ESP_ERR_NO_MEM;
        }
        idx = s_table.count++;
        strlcpy(s_table.entries[idx].mac, norm, sizeof(s_table.entries[idx].mac));
    }
    strlcpy(s_table.entries[idx].ip, ip_str, sizeof(s_table.entries[idx].ip));
    esp_err_t err = persist();
    unlock();

    ESP_LOGI(TAG, "%s %s -> %s", err == ESP_OK ? "Stored" : "Failed to store",
             norm, ip_str[0] ? ip_str : "(default)");
    return err;
}

esp_err_t mac_store_remove(const char *mac)
{
    char norm[WM_CFG_MAC_LEN];
    if (!normalise_mac(mac, norm, sizeof(norm))) {
        return ESP_ERR_INVALID_ARG;
    }

    lock();
    int idx = find_index(norm);
    if (idx < 0) {
        unlock();
        return ESP_ERR_NOT_FOUND;
    }
    for (int i = idx; i < s_table.count - 1; i++) {
        s_table.entries[i] = s_table.entries[i + 1];
    }
    s_table.count--;
    memset(&s_table.entries[s_table.count], 0, sizeof(s_table.entries[0]));
    esp_err_t err = persist();
    unlock();

    if (err == ESP_OK) {
        ESP_LOGI(TAG, "Removed %s", norm);
    }
    return err;
}

esp_err_t mac_store_clear(void)
{
    lock();
    memset(&s_table, 0, sizeof(s_table));
    esp_err_t err = persist();
    unlock();

    if (err == ESP_OK) {
        ESP_LOGI(TAG, "Address book cleared");
    }
    return err;
}