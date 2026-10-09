idf_component_register(SRCS "control_mqtt.c"
                       INCLUDE_DIRS "include"
                       REQUIRES app_config config_store device_info wol lwip esp_netif esp_wifi)