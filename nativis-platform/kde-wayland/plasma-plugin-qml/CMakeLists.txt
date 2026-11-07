cmake_minimum_required(VERSION 3.16)
project(nativis_qml_plugin_wayland VERSION 2.0.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_BUILD_TYPE Release CACHE STRING "" FORCE)
add_compile_options(-O2)
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)

find_package(Qt6 COMPONENTS Core Gui Qml Quick OpenGL REQUIRED)

find_program(QSB_EXECUTABLE NAMES qsb-qt6 qsb REQUIRED)

set(SHADER_DIR "${CMAKE_CURRENT_SOURCE_DIR}/shaders")
set(VERT_SRC "${SHADER_DIR}/nv12.vert")
set(FRAG_SRC "${SHADER_DIR}/nv12.frag")
set(VERT_QSB "${SHADER_DIR}/nv12.vert.qsb")
set(FRAG_QSB "${SHADER_DIR}/nv12.frag.qsb")

add_custom_command(
    OUTPUT ${VERT_QSB}
    COMMAND ${QSB_EXECUTABLE} -b --glsl "100 es,120,330" --hlsl 50 --msl 12 -o ${VERT_QSB} ${VERT_SRC}
    DEPENDS ${VERT_SRC}
    COMMENT "Compiling NV12 vertex shader with qsb (batchable)"
)

add_custom_command(
    OUTPUT ${FRAG_QSB}
    COMMAND ${QSB_EXECUTABLE} --glsl "100 es,120,330" --hlsl 50 --msl 12 -o ${FRAG_QSB} ${FRAG_SRC}
    DEPENDS ${FRAG_SRC}
    COMMENT "Compiling NV12 fragment shader with qsb"
)

set(PLUGIN_NAME nativisplugin)
set(QML_IMPORT_DIR "org/nativis")

add_library(${PLUGIN_NAME} SHARED
    nativisplugin.cpp
    nativisitem.cpp
    shaders/shaders.qrc
    ${VERT_QSB}
    ${FRAG_QSB}
)

target_link_libraries(${PLUGIN_NAME} PRIVATE
    Qt6::Core
    Qt6::Gui
    Qt6::Qml
    Qt6::Quick
    Qt6::OpenGL
)

target_link_libraries(${PLUGIN_NAME} PRIVATE
    ${CMAKE_SOURCE_DIR}/../../../target/release/libnativis_v2_core.so
)

set_target_properties(${PLUGIN_NAME} PROPERTIES
    INSTALL_RPATH "$ORIGIN"
    BUILD_WITH_INSTALL_RPATH TRUE
)

add_custom_command(TARGET ${PLUGIN_NAME} POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E make_directory ${CMAKE_BINARY_DIR}/${QML_IMPORT_DIR}
    COMMAND ${CMAKE_COMMAND} -E copy $<TARGET_FILE:${PLUGIN_NAME}> ${CMAKE_BINARY_DIR}/${QML_IMPORT_DIR}/
    COMMAND ${CMAKE_COMMAND} -E copy ${CMAKE_SOURCE_DIR}/qmldir.in ${CMAKE_BINARY_DIR}/${QML_IMPORT_DIR}/qmldir
)
