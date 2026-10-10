idf_component_register(SRCS "control_mqtt.c"
                       INCLUDE_DIRS "include"
                       REQUIRES app_config config_store device_info led_status wol mac_store chip_temp time_sync lwip esp_netif esp_wifi)