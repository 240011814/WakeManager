idf_component_register(SRCS "wifi_prov.c"
                       INCLUDE_DIRS "include"
                       REQUIRES app_config led_status nvs_flash esp_wifi esp_event esp_netif)