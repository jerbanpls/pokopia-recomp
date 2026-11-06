# SPDX-License-Identifier: GPL-3.0-or-later
# The pre-game launcher (apps/launcher): plain Win32, static runtime, no installer. It edits controls.ini with the same settings code the game uses
# (src/gfx/play/play_types.cpp, play_config.cpp; SDL3 is linked only for its key name table), starts the setup tool for the first-time setup
# and starts the game exe. Included from gfx.cmake (needs the SDL3 target). Its branding and factory settings are those of one game module
# (NXRT_LAUNCHER_GAME): the data library (names, controls.ini defaults) and its launcher.rc (version info, manifest).
set(NXRT_LAUNCHER_GAME "mpr" CACHE STRING "id of the game module whose launcher this is (games/<id>/launcher.rc, nxrt_game_<id>_data)")
set(NXRT_LAUNCHER_DIR "${CMAKE_CURRENT_SOURCE_DIR}/apps/launcher")
add_library(mpr_launcher_core STATIC
    "${NXRT_LAUNCHER_DIR}/keymap_win.cpp" "${NXRT_LAUNCHER_DIR}/game_info.cpp" "${NXRT_LAUNCHER_DIR}/bindings_edit.cpp"
    "${NXRT_LAUNCHER_DIR}/pad_capture.cpp"
    "${CMAKE_CURRENT_SOURCE_DIR}/src/gfx/play/play_types.cpp" "${CMAKE_CURRENT_SOURCE_DIR}/src/gfx/play/play_config.cpp"
    "${CMAKE_CURRENT_SOURCE_DIR}/src/gfx/play/play_fastboot.cpp")
target_include_directories(mpr_launcher_core PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}/include" "${CMAKE_CURRENT_SOURCE_DIR}/src/gfx" "${NXRT_LAUNCHER_DIR}")
target_compile_definitions(mpr_launcher_core PUBLIC UNICODE _UNICODE)
target_link_libraries(mpr_launcher_core PUBLIC SDL3::SDL3-static shell32 shlwapi)

add_executable(mpr_launcher WIN32
    "${NXRT_LAUNCHER_DIR}/main.cpp" "${NXRT_LAUNCHER_DIR}/ui.cpp" "${NXRT_LAUNCHER_DIR}/page_display.cpp" "${NXRT_LAUNCHER_DIR}/page_controls.cpp"
    "${NXRT_LAUNCHER_DIR}/page_keys.cpp" "${NXRT_LAUNCHER_DIR}/page_pad.cpp" "${NXRT_LAUNCHER_DIR}/page_fastboot.cpp"
    "${NXRT_LAUNCHER_DIR}/page_tweaks.cpp")
target_link_libraries(mpr_launcher PRIVATE mpr_launcher_core comctl32 uxtheme dwmapi shlwapi ole32 shell32)
# The game module's part (launcher.rc, nxrt_game_<id>_data, the register call) is added in the top CMakeLists.txt once games/ is configured.
# The release version the launcher shows (set by the game build: game-build/CMakeLists.txt, -DMPR_VERSION=0.1.0).
if(NOT DEFINED MPR_VERSION)
    set(MPR_VERSION "dev")
endif()
set_source_files_properties("${NXRT_LAUNCHER_DIR}/main.cpp" PROPERTIES COMPILE_DEFINITIONS "MPR_VERSION=\"${MPR_VERSION}\"")
set_target_properties(mpr_launcher PROPERTIES RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}")   # next to mpr.exe in the game build
