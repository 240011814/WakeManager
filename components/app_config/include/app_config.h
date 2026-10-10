/*
 * WakeManager - build-time / user tunable configuration
 *
 * Edit the values here (or move them to menuconfig later) to match your
 * network and targets.
 */

#pragma once

/* ---------- Wake-on-LAN ---------- */

/* UDP port for the magic packet (9 = classic wakeonlan, 7 also common) */
#define WOL_UDP_PORT            9

/* Destination of a magic packet is decided at send time:
 *   - an explicit IP in the wake command (HTTP ?ip= / MQTT "ip"), else
 *   - the IP stored in the address book for the target MAC, else
 *   - the subnet-directed broadcast derived from this device's own IP and
 *     netmask, e.g. device 192.168.1.88/24 -> 192.168.1.255 (falling back to
 *     255.255.255.255 when no usable address is available).
 * There is no static broadcast setting. */

/* ---------- HTTP control server ---------- */

#define WOL_HTTP_PORT           80
#define WOL_HTTP_ENABLE         1

/* ---------- MQTT control ---------- */

#define WOL_MQTT_ENABLE         1
#define WOL_MQTT_BROKER_URI     "mqtt://demonkingqianjing.cloud:21883"
#define WOL_MQTT_CLIENT_ID      "wakemanager"
#define WOL_MQTT_USERNAME       ""      /* leave empty for anonymous */
#define WOL_MQTT_PASSWORD       ""      /* leave empty for anonymous */

/* MQTT topics are fixed at build time (not user configurable).
 *
 * Wake command topic. Each device subscribes to "<topic>/<device_id>", where
 * <device_id> is the Wi-Fi station MAC as 12 hex chars (e.g. A1B2C3D4E5F6), so
 * wake commands are addressed to a single device:
 *   {"cmd":"wake","mac":"AA:BB:CC:DD:EE:FF","ip":"192.168.1.255"}
 * "mac" is required; "ip" is optional and, when omitted, falls back to the IP
 * stored in the address book for that MAC, then to the default destination
 * (the configured broadcast, or the subnet broadcast derived from this
 * device's IP). */
#define WOL_MQTT_CMD_TOPIC      "wakemanager/cmd"
/* Shared broadcast topic. Every device subscribes to it, so a remote that does
 * not yet know any device id can publish a discovery command here and collect
 * the replies (each reply carries the device id) from the status topic:
 *   {"cmd":"status"}
 * A missing "cmd" is also treated as a status request. Wake commands are NOT
 * accepted here; they must go to "<cmd_topic>/<device_id>". */
#define WOL_MQTT_BROADCAST_TOPIC "wakemanager/broadcast"
/* Device publishes its results/status here */
#define WOL_MQTT_STATUS_TOPIC   "wakemanager/status"

/* MAC/IP address book topic. Per-device: the device subscribes to
 * "<topic>/<device_id>" and accepts:
 *   {"cmd":"set","mac":"AA:BB:CC:DD:EE:FF","ip":"192.168.1.5"}
 *   {"cmd":"del","mac":"AA:BB:CC:DD:EE:FF"}     (aliases: delete/remove)
 *   {"cmd":"clear"}
 * At most WM_MAC_STORE_MAX (10) entries are kept. Results go to the status
 * topic. */
#define WOL_MQTT_MAC_TOPIC      "wakemanager/mac"
/* MAC/IP address book query topic. Per-device: the device subscribes to
 * "<topic>/<device_id>" and replies on the status topic with the full list:
 *   {"cmd":"get"}     (a missing/empty "cmd" also triggers the reply) */
#define WOL_MQTT_MAC_GET_TOPIC  "wakemanager/mac/get"

/* ---------- Time sync (SNTP) ---------- */

/* NTP server used to calibrate the clock after Wi-Fi connects. */
#define NTP_SERVER              "pool.ntp.org"
/* POSIX timezone string (e.g. "CST-8" for UTC+8, "UTC0" for UTC). */
#define TIME_TIMEZONE           "CST-8"

/* ---------- Watchdog ---------- */

/* Task watchdog timeout. Subscribed tasks must feed it within this window or,
 * when WATCHDOG_TRIGGER_PANIC is 1, the device panics and reboots. */
#define WATCHDOG_TIMEOUT_MS     30000
#define WATCHDOG_TRIGGER_PANIC  1

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