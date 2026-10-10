idf_component_register(SRCS "chip_temp.c"
                       INCLUDE_DIRS "include"
                       REQUIRES esp_driver_tsens log)