idf_component_register(SRCS "mac_store.c"
                       INCLUDE_DIRS "include"
                       REQUIRES config_store wol nvs_flash lwip)