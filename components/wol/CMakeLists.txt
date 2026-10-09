idf_component_register(SRCS "wol.c"
                       INCLUDE_DIRS "include"
                       REQUIRES app_config config_store lwip esp_netif)