/*
 * WakeManager - status indicator LED
 *
 * Drives a simple (on/off) LED to show the Wi-Fi connection state:
 *   - LED_STATUS_CONNECTING : slow blink
 *   - LED_STATUS_CONNECTED  : solid on
 */

#pragma once

#ifdef __cplusplus
extern "C" {
#endif

typedef enum {
    LED_STATUS_CONNECTING = 0,  /* device is provisioning / (re)connecting */
    LED_STATUS_CONNECTED,       /* connected with an IP address */
} led_status_t;

/**
 * @brief Configure the indicator GPIO and start the LED driver task.
 */
void led_status_init(void);

/**
 * @brief Update the indicated Wi-Fi state.
 */
void led_status_set(led_status_t status);

#ifdef __cplusplus
}
#endif