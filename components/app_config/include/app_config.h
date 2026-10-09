/*
 * WakeManager - build-time / user tunable configuration
 *
 * Edit the values here (or move them to menuconfig later) to match your
 * network and targets.
 */

#pragma once

/* ---------- Wake-on-LAN ---------- */

/* Default target woken when no MAC is supplied in a request */
#define WOL_DEFAULT_MAC         "AA:BB:CC:DD:EE:FF"

/* UDP port for the magic packet (9 = classic wakeonlan, 7 also common) */
#define WOL_UDP_PORT            9

/* Where the magic packet is sent. When left empty (or "255.255.255.255") the
 * subnet-directed broadcast is derived automatically from this device's own
 * IP and netmask, e.g. device 192.168.1.88/24 -> 192.168.1.255. Set an explicit
 * address here (e.g. "192.168.1.255") only to override the automatic value. */
#define WOL_BROADCAST_ADDR      "255.255.255.255"

/* ---------- HTTP control server ---------- */

#define WOL_HTTP_PORT           80
#define WOL_HTTP_ENABLE         1

/* ---------- MQTT control ---------- */

#define WOL_MQTT_ENABLE         1
#define WOL_MQTT_BROKER_URI     "mqtt://demonkingqianjing.cloud:21883"
#define WOL_MQTT_CLIENT_ID      "wakemanager"
#define WOL_MQTT_USERNAME       ""      /* leave empty for anonymous */
#define WOL_MQTT_PASSWORD       ""      /* leave empty for anonymous */
/* A message on this topic triggers a wake and must be a JSON object:
 *   {"mac":"AA:BB:CC:DD:EE:FF","ip":"192.168.1.255"}
 * Both fields are optional: omit "mac" to use the default target, omit "ip"
 * to use the default destination (the configured broadcast, or the subnet
 * broadcast auto-derived from this device's IP). */
#define WOL_MQTT_CMD_TOPIC      "wakemanager/cmd"
/* Device publishes its results/status here */
#define WOL_MQTT_STATUS_TOPIC   "wakemanager/status"

/* ---------- Wi-Fi provisioning (BLE transport) ---------- */

/* Proof-of-possession (PoP). The provisioning client must supply this same
 * string to open a secure session. Change it for production and ideally make
 * it unique per device (e.g. derived from the MAC or a manufacturing secret). */
#define PROV_POP                "12345678"

/* BLE device name / service name prefix. The advertised name is
 * "<prefix><last 3 bytes of MAC>", e.g. "PROV_A1B2C3". Keep it short:
 * legacy BLE advertisements are limited to 31 bytes. */
#define PROV_DEVICE_NAME_PREFIX "PROV_"

/* Metadata used when printing the provisioning QR payload (log only). */
#define PROV_QR_VERSION         "v1"
#define PROV_QR_TRANSPORT       "ble"
#define PROV_QRCODE_BASE_URL    "https://espressif.github.io/esp-jumpstart/qrcode.html"

/* Note: the transport (BLE) and the protocomm security scheme (Security 1) are
 * selected in components/wifi_prov/wifi_prov.c, because switching them also
 * changes the code path (scheme object, security params and Kconfig options). */