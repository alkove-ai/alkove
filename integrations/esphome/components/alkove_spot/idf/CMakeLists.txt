# the ESP-IDF side of the component: the Rust library (prebuilt), its header, the model and the test audio embedded 16-byte aligned (blobs.S)
idf_component_register(SRCS "blobs.S" INCLUDE_DIRS "include" PRIV_REQUIRES espressif__esp-nn)
# .incbin searches the assembler's own include path
target_compile_options(${COMPONENT_LIB} PRIVATE "-Wa,-I${CMAKE_CURRENT_SOURCE_DIR}/data")
set_property(SOURCE blobs.S APPEND PROPERTY OBJECT_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/data/model.mcu" "${CMAKE_CURRENT_SOURCE_DIR}/data/test.pcm")
add_prebuilt_library(alkove_spot_esp "${CMAKE_CURRENT_SOURCE_DIR}/lib/libalkove_spot_esp.a" PRIV_REQUIRES espressif__esp-nn pthread newlib)
target_link_libraries(${COMPONENT_LIB} INTERFACE alkove_spot_esp)
