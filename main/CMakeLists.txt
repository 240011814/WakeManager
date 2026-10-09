idf_component_register(SRCS "app_main.c"
                       PRIV_REQUIRES spi_flash app_config led_status wifi_prov wol_http wol_mqtt
                       INCLUDE_DIRS "")