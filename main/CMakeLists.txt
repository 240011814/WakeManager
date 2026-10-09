idf_component_register(SRCS "app_main.c"
                       PRIV_REQUIRES spi_flash nvs_flash app_config config_store led_status wifi_prov wol_http wol_mqtt
                       INCLUDE_DIRS "")