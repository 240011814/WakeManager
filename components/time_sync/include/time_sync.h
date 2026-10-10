/*
 * WakeManager - SNTP clock calibration
 *
 * Started once Wi-Fi has an IP address. The clock is synchronised with the
 * configured NTP server and the POSIX timezone is applied so local time can be
 * reported (status messages / web UI).
 */

#pragma once

#include <stdbool.h>
#include <stddef.h>

#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

/**
 * @brief Configure the timezone and start SNTP polling.
 *
 * Non-blocking: the actual sync happens asynchronously. Call after the device
 * has network connectivity.
 */
esp_err_t time_sync_start(void);

/**
 * @brief Whether the clock has been synchronised at least once.
 */
bool time_sync_is_synced(void);

/**
 * @brief Format the current local time as "YYYY-MM-DD HH:MM:SS".
 *
 * @param[out] out      Destination buffer
 * @param[in]  out_len  Size of @p out (at least 20 bytes)
 */
void time_sync_format(char *out, size_t out_len);

#ifdef __cplusplus
}
#endif