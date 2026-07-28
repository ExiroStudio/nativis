cmake_minimum_required(VERSION 3.16)
project(nativis_qml_plugin VERSION 1.0.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)

find_package(Qt5 COMPONENTS Core Gui Qml Quick REQUIRED)

# The plugin module name
set(PLUGIN_NAME nativisplugin)
set(QML_IMPORT_DIR "org/nativis")

add_library(${PLUGIN_NAME} SHARED
    nativisplugin.cpp
    nativisitem.cpp
)

target_link_libraries(${PLUGIN_NAME} PRIVATE
    Qt5::Core
    Qt5::Gui
    Qt5::Qml
    Qt5::Quick
)

# Link against the Rust runtime
target_link_libraries(${PLUGIN_NAME} PRIVATE
    ${CMAKE_SOURCE_DIR}/../../../target/debug/libnativis_v1_core.so
)

set_target_properties(${PLUGIN_NAME} PROPERTIES
    INSTALL_RPATH "${CMAKE_SOURCE_DIR}/../../../target/debug"
    BUILD_WITH_INSTALL_RPATH TRUE
)



# Copy qmldir and the built plugin to the build output directory matching the QML import path
add_custom_command(TARGET ${PLUGIN_NAME} POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E make_directory ${CMAKE_BINARY_DIR}/${QML_IMPORT_DIR}
    COMMAND ${CMAKE_COMMAND} -E copy $<TARGET_FILE:${PLUGIN_NAME}> ${CMAKE_BINARY_DIR}/${QML_IMPORT_DIR}/
    COMMAND ${CMAKE_COMMAND} -E copy ${CMAKE_SOURCE_DIR}/qmldir.in ${CMAKE_BINARY_DIR}/${QML_IMPORT_DIR}/qmldir
)
