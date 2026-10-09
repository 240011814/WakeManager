/*
 * WakeManager - device identity helpers
 */

#include "device_info.h"

#include <stdio.h>

#include "esp_wifi.h"

bool device_info_get_mac(uint8_t mac[6])
{
    if (mac == NULL) {
        return false;
    }
    return esp_wifi_get_mac(WIFI_IF_STA, mac) == ESP_OK;
}

void device_info_get_id(char *out, size_t out_len)
{
    if (out == NULL || out_len == 0) {
        return;
    }

    uint8_t mac[6] = {0};
    device_info_get_mac(mac);

    snprintf(out, out_len, "%02X%02X%02X%02X%02X%02X",
             mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
}