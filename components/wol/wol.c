/*
 * WakeManager - Wake-on-LAN magic packet sender
 *
 * A magic packet is 6 bytes of 0xFF followed by the target MAC repeated
 * 16 times (102 bytes total), sent as a UDP broadcast.
 */

#include "wol.h"

#include <string.h>
#include <stdio.h>
#include <ctype.h>

#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <unistd.h>

#include <esp_log.h>
#include <esp_netif.h>

#include "app_config.h"
#include "config_store.h"

static const char *TAG = "wol";

#define MAGIC_PACKET_LEN    (6 + 6 * 16)

/* Value used when the subnet broadcast cannot be derived. */
#define WOL_LIMITED_BROADCAST   "255.255.255.255"

/*
 * Derive the subnet-directed broadcast address from this device's own IPv4
 * configuration, e.g. an interface at 192.168.1.88/24 yields 192.168.1.255.
 *
 * This is computed from IP & netmask (so it works for any prefix length, not
 * only /24). Falls back to the limited broadcast 255.255.255.255 when the
 * station interface has no usable IPv4 address (e.g. not connected yet).
 */
static void wol_auto_broadcast(struct in_addr *out)
{
    out->s_addr = htonl(INADDR_BROADCAST); /* 255.255.255.255 */

    esp_netif_t *netif = esp_netif_get_handle_from_ifkey("WIFI_STA_DEF");
    if (netif == NULL) {
        return;
    }

    esp_netif_ip_info_t ip_info;
    if (esp_netif_get_ip_info(netif, &ip_info) != ESP_OK ||
        ip_info.ip.addr == 0 || ip_info.netmask.addr == 0) {
        return;
    }

    /* directed broadcast = (ip & netmask) | ~netmask (network byte order) */
    out->s_addr = (ip_info.ip.addr & ip_info.netmask.addr) | ~ip_info.netmask.addr;
}

void wol_get_default_broadcast(char *out, size_t out_len)
{
    if (out == NULL || out_len == 0) {
        return;
    }

    struct in_addr addr;
    wol_auto_broadcast(&addr);
    if (inet_ntop(AF_INET, &addr, out, out_len) == NULL) {
        snprintf(out, out_len, "%s", WOL_LIMITED_BROADCAST);
    }
}

bool wol_parse_mac(const char *str, uint8_t mac[6])
{
    if (str == NULL || mac == NULL || *str == '\0') {
        return false;
    }

    unsigned int b[6] = {0};
    int n = sscanf(str, "%x:%x:%x:%x:%x:%x", &b[0], &b[1], &b[2], &b[3], &b[4], &b[5]);
    if (n != 6) {
        n = sscanf(str, "%x-%x-%x-%x-%x-%x", &b[0], &b[1], &b[2], &b[3], &b[4], &b[5]);
    }
    if (n != 6 && strlen(str) == 12) {
        n = sscanf(str, "%2x%2x%2x%2x%2x%2x", &b[0], &b[1], &b[2], &b[3], &b[4], &b[5]);
    }
    if (n != 6) {
        return false;
    }

    for (int i = 0; i < 6; i++) {
        if (b[i] > 0xFF) {
            return false;
        }
        mac[i] = (uint8_t)b[i];
    }
    return true;
}

void wol_mac_to_str(const uint8_t mac[6], char *out, size_t out_len)
{
    if (out == NULL || out_len == 0) {
        return;
    }
    snprintf(out, out_len, "%02X:%02X:%02X:%02X:%02X:%02X",
             mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
}

esp_err_t wol_send_to(const uint8_t mac[6], const char *dest_ip)
{
    uint8_t packet[MAGIC_PACKET_LEN];

    memset(packet, 0xFF, 6);
    for (int i = 0; i < 16; i++) {
        memcpy(&packet[6 + i * 6], mac, 6);
    }

    wm_config_t cfg;
    config_store_get(&cfg);
    uint16_t port = (cfg.wol_udp_port != 0) ? cfg.wol_udp_port : WOL_UDP_PORT;

    /* Destination address: an explicit IP from the command overrides; when it
     * is empty, fall back to the subnet broadcast derived from this device's
     * own IP and netmask. */
    struct in_addr dest_addr;
    if (dest_ip != NULL && dest_ip[0] != '\0') {
        if (inet_pton(AF_INET, dest_ip, &dest_addr) != 1) {
            ESP_LOGE(TAG, "Invalid destination IP: %s", dest_ip);
            return ESP_ERR_INVALID_ARG;
        }
    } else {
        wol_auto_broadcast(&dest_addr);
    }

    char bcast[INET_ADDRSTRLEN];
    inet_ntop(AF_INET, &dest_addr, bcast, sizeof(bcast));

    int sock = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    if (sock < 0) {
        ESP_LOGE(TAG, "socket() failed: errno %d", errno);
        return ESP_FAIL;
    }

    int broadcast = 1;
    setsockopt(sock, SOL_SOCKET, SO_BROADCAST, &broadcast, sizeof(broadcast));

    struct sockaddr_in dest = {
        .sin_family = AF_INET,
        .sin_port = htons(port),
        .sin_addr = dest_addr,
    };

    ssize_t sent = sendto(sock, packet, sizeof(packet), 0,
                          (struct sockaddr *)&dest, sizeof(dest));
    int err = errno;
    close(sock);

    if (sent != (ssize_t)sizeof(packet)) {
        ESP_LOGE(TAG, "sendto() failed: sent=%d errno=%d", (int)sent, err);
        return ESP_FAIL;
    }

    char mac_str[18];
    wol_mac_to_str(mac, mac_str, sizeof(mac_str));
    ESP_LOGI(TAG, "Magic packet sent to %s (%s:%d)", mac_str, bcast, port);
    return ESP_OK;
}

esp_err_t wol_send(const uint8_t mac[6])
{
    return wol_send_to(mac, NULL);
}

esp_err_t wol_send_str_to(const char *mac_str, const char *dest_ip)
{
    uint8_t mac[6];
    if (!wol_parse_mac(mac_str, mac)) {
        ESP_LOGE(TAG, "Invalid MAC address: %s", mac_str ? mac_str : "(null)");
        return ESP_ERR_INVALID_ARG;
    }
    return wol_send_to(mac, dest_ip);
}

esp_err_t wol_send_str(const char *mac_str)
{
    return wol_send_str_to(mac_str, NULL);
}