/*
 * WakeManager - Task Watchdog (TWDT) control
 *
 * Enables the ESP-IDF task watchdog with a project-specific timeout and panic
 * behaviour. On boot the idle tasks are monitored automatically (so a task that
 * starves the scheduler is caught); callers can additionally subscribe their
 * own task and feed it from their main loop.
 *
 * The interrupt watchdog is enabled separately through Kconfig
 * (CONFIG_ESP_INT_WDT, on by default) and needs no runtime setup here.
 */

#pragma once

#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

/**
 * @brief Configure and enable the task watchdog.
 *
 * Reconfigures the watchdog when it was already auto-initialized by the IDF
 * (CONFIG_ESP_TASK_WDT_INIT), otherwise initializes it. Monitors all idle
 * tasks. Call after the scheduler has started.
 */
esp_err_t watchdog_init(void);

/**
 * @brief Subscribe the calling task to the watchdog.
 *
 * The task must then call watchdog_feed() regularly.
 */
esp_err_t watchdog_add_current_task(void);

/**
 * @brief Feed the watchdog on behalf of the calling task.
 */
void watchdog_feed(void);

#ifdef __cplusplus
}
#endif