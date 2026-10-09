idf_component_register(SRCS "config_store.c"
                       INCLUDE_DIRS "include"
                       REQUIRES app_config nvs_flash)