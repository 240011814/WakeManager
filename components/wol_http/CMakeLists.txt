idf_component_register(SRCS "control_http.c"
                       INCLUDE_DIRS "include"
                       REQUIRES app_config wol esp_http_server)