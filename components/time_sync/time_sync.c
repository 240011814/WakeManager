/*
 * WakeManager - SNTP clock calibration
 */

#include "time_sync.h"

#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <sys/time.h>

#include "esp_log.h"
#include "esp_sntp.h"

#include "app_config.h"

static const char *TAG = "time_sync";

static volatile bool s_synced = false;

static void sync_notification_cb(struct timeval *tv)
{
    s_synced = true;

    time_t now = tv->tv_sec;
    struct tm local;
    localtime_r(&now, &local);

    char buf[32];
    strftime(buf, sizeof(buf), "%Y-%m-%d %H:%M:%S", &local);
    ESP_LOGI(TAG, "Clock synchronised: %s", buf);
}

esp_err_t time_sync_start(void)
{
    /* Apply the configured timezone before the first sync so localtime_r()
     * already returns local time once SNTP completes. */
    setenv("TZ", TIME_TIMEZONE, 1);
    tzset();

    esp_sntp_setoperatingmode(ESP_SNTP_OPMODE_POLL);
    esp_sntp_setservername(0, NTP_SERVER);
    sntp_set_time_sync_notification_cb(sync_notification_cb);
    esp_sntp_init();

    ESP_LOGI(TAG, "SNTP started (server=%s, tz=%s)", NTP_SERVER, TIME_TIMEZONE);
    return ESP_OK;
}

bool time_sync_is_synced(void)
{
    return s_synced;
}

void time_sync_format(char *out, size_t out_len)
{
    if (out == NULL || out_len == 0) {
        return;
    }

    time_t now = time(NULL);
    struct tm local;
    localtime_r(&now, &local);
    strftime(out, out_len, "%Y-%m-%d %H:%M:%S", &local);
}