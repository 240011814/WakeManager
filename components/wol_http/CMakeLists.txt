idf_component_register(SRCS "control_http.c"
                       INCLUDE_DIRS "include"
                       REQUIRES app_config config_store device_info wol wol_mqtt esp_http_server esp_wifi esp_netif
                       EMBED_TXTFILES "index.html")