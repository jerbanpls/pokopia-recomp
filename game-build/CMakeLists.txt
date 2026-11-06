# SPDX-License-Identifier: GPL-3.0-or-later
# pkp.exe: the runtime (../runtime) + the four lifted game libraries (built by ../recomp/scripts/build_game.cmd pkp into ../work/build/pkp).
#   pwsh scripts\build.ps1            configure + build into game-build\build\release
# Nothing derived from the game lives here: the libraries are imported from the work folder, the NSOs are read at run time.
cmake_minimum_required(VERSION 3.25)
# Static MSVC runtime (/MT): players need no Visual C++ Redistributable. Set before project() so every target below, third-party code
# included, uses it; the libraries this one links must agree (a mismatch is a link error).
if(NOT DEFINED CMAKE_MSVC_RUNTIME_LIBRARY)
    set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
endif()
project(pkp_game C CXX)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

get_filename_component(PKP_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/.." ABSOLUTE)
set(PKP_LIB_DIR "${PKP_ROOT}/work/build/pkp" CACHE PATH "folder with pkp_main.lib pkp_sdk.lib pkp_subsdk0.lib pkp_rtld.lib (built by recomp scripts build_game.cmd pkp)")
set(PKP_EXEFS_DEFAULT "${PKP_ROOT}/game/dump/exefs" CACHE PATH "default ExeFS folder baked into pkp.exe (--exefs / PKP_EXEFS override it)")

# Release packages (PKP_RELEASE=ON): pkp.exe is a Windows GUI program (double click opens the game window only, no console; errors show in
# a message box, output still reaches a terminal it was started from), looks for ./exefs by default, and carries no local build paths
# (source paths in assert strings, the .pdb path): see the installed-layout comment in src/pkp_main.cpp.
# __FILE__ (the assert strings of the audio renderer) never carries the build folder, in any build: paths are written relative to the checkout.
add_compile_options("-clang:-fmacro-prefix-map=${PKP_ROOT}=.")
option(PKP_RELEASE "build pkp.exe as a release package binary" OFF)
if(PKP_RELEASE)
    set(PKP_GUI_SUBSYSTEM ON)
    set(PKP_EXEFS_DEFAULT "exefs")
    add_compile_options("-clang:-ffile-prefix-map=${PKP_ROOT}=.")
endif()

# The release version ("0.1.0"; "dev" for local builds). The game log, --version and the exe's file properties show it, and the
# launcher (built from the runtime with the same cache value) shows it in its window. game-build/scripts/build.ps1 -Version sets it.
set(PKP_VERSION "dev" CACHE STRING "release version shown by pkp.exe and the launcher")
set(PKP_VER_MAJOR 0)
set(PKP_VER_MINOR 0)
set(PKP_VER_PATCH 0)
if(PKP_VERSION MATCHES "^([0-9]+)[.]([0-9]+)[.]([0-9]+)")
    set(PKP_VER_MAJOR ${CMAKE_MATCH_1})
    set(PKP_VER_MINOR ${CMAKE_MATCH_2})
    set(PKP_VER_PATCH ${CMAKE_MATCH_3})
endif()
# PKP_RUNTIME_DIR: which runtime checkout compiles in (default the main one; a worktree such as runtime-wt/boot2 for branch work)
set(PKP_RUNTIME_DIR "${PKP_ROOT}/runtime" CACHE PATH "runtime source tree compiled into pkp.exe")
set(NXRT_GAMES "pkp" CACHE STRING "" FORCE)            # runtime/games/pkp only
set(NXRT_LAUNCHER_GAME "pkp" CACHE STRING "" FORCE)
set(NXRT_RECOMP_INCLUDE "${PKP_ROOT}/recomp/include" CACHE PATH "")
add_subdirectory("${PKP_RUNTIME_DIR}" runtime)

foreach(m main sdk subsdk0 rtld)
    add_library(pkp_${m} STATIC IMPORTED GLOBAL)
    set_target_properties(pkp_${m} PROPERTIES IMPORTED_LOCATION "${PKP_LIB_DIR}/pkp_${m}.lib")
endforeach()

# Release model (b) (PKP_RT_DLL=ON, docs/RELEASE.md): no game code in anything built here. The runtime and the boot code become
# pkp_rt.dll (+ the import library pkp_rt.lib); pkp.exe is linked on the player's PC from the libraries lifted out of their own dump
# and src/pkp_stub.cpp (tools/build/pkp_build.py in the package, with the bundled Zig). Nothing lifted is imported here.
option(PKP_RT_DLL "build the runtime as pkp_rt.dll for release model (b)" OFF)
if(PKP_RT_DLL)
    add_library(pkp_rt SHARED src/pkp_main.cpp src/pkp_rt.def)
    set_source_files_properties(src/pkp_main.cpp PROPERTIES COMPILE_DEFINITIONS "PKP_VERSION=\"${PKP_VERSION}\"")
    target_compile_definitions(pkp_rt PRIVATE PKP_RT_DLL=1 PKP_GUI_SUBSYSTEM=1 PKP_EXEFS_DEFAULT="exefs")
    target_link_libraries(pkp_rt PRIVATE nxrt_game_pkp nxrt_gfx nxrt)
    # /DEBUG off: no .pdb path in the DLL. Low host allocations as for pkp.exe (guest ranges at 0x71_0000_0000 stay free).
    target_link_options(pkp_rt PRIVATE /HIGHENTROPYVA:NO /DEBUG:NONE /Brepro)
    target_compile_options(pkp_rt PRIVATE "-clang:-ffile-prefix-map=${PKP_ROOT}=.")
    add_custom_command(TARGET pkp_rt POST_BUILD COMMAND ${CMAKE_COMMAND} -E copy_if_different
        "${PKP_RUNTIME_DIR}/assets/fonts/DejaVuSans.ttf" "$<TARGET_FILE_DIR:pkp_rt>/fonts/DejaVuSans.ttf")
    file(GLOB PKP_A2B_DLLS "${NXRT_A2B_DIR}/astcdec-*-shared.dll")
    foreach(dll ${PKP_A2B_DLLS})
        add_custom_command(TARGET pkp_rt POST_BUILD COMMAND ${CMAKE_COMMAND} -E copy_if_different "${dll}" "$<TARGET_FILE_DIR:pkp_rt>")
    endforeach()
    return()
endif()

add_executable(pkp src/pkp_main.cpp res/pkp.rc)
set_source_files_properties(src/pkp_main.cpp PROPERTIES COMPILE_DEFINITIONS "PKP_VERSION=\"${PKP_VERSION}\"")
set_source_files_properties(res/pkp.rc PROPERTIES COMPILE_DEFINITIONS "PKP_VER_MAJOR=${PKP_VER_MAJOR};PKP_VER_MINOR=${PKP_VER_MINOR};PKP_VER_PATCH=${PKP_VER_PATCH};PKP_VER_TEXT=\"${PKP_VERSION}\""
                            OBJECT_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/res/pkp.manifest")
target_compile_definitions(pkp PRIVATE PKP_EXEFS_DEFAULT="${PKP_EXEFS_DEFAULT}")
target_link_libraries(pkp PRIVATE pkp_main pkp_sdk pkp_subsdk0 pkp_rtld nxrt_game_pkp nxrt_gfx nxrt)   # nxrt_game_pkp: runtime/games/pkp
# A linker map next to the exe (host frames get names), a big main stack (lifted
# code nests deeply), low host allocations so the guest ranges at 0x71_0000_0000 and up stay free.
if(PKP_RELEASE)
    target_compile_definitions(pkp PRIVATE PKP_GUI_SUBSYSTEM=1)
    target_link_options(pkp PRIVATE /SUBSYSTEM:WINDOWS /ENTRY:mainCRTStartup /PDBALTPATH:pkp.pdb)
endif()
target_link_options(pkp PRIVATE /MAP /STACK:134217728 /HIGHENTROPYVA:NO /DEBUG /MANIFEST:NO)   # res/pkp.rc carries the manifest

# The ASTC decoder DLLs astc2bc7 loads at run time (NVN textures in ASTC are transcoded to BC7 on first use): next to pkp.exe.
# (NXRT_A2B_DIR is the tools/textures build folder the runtime links its transcoder from: cmake/gfx.cmake)
# The shared-font fallback (runtime src/services/pl.cpp looks in fonts\ next to the exe): without any font the game crashes at boot.
add_custom_command(TARGET pkp POST_BUILD COMMAND ${CMAKE_COMMAND} -E copy_if_different
    "${PKP_RUNTIME_DIR}/assets/fonts/DejaVuSans.ttf" "$<TARGET_FILE_DIR:pkp>/fonts/DejaVuSans.ttf")

# Keyboard and mouse prompt icons (runtime games/pkp/pkp_kbm.cpp reads assets\kbm_icons\<name>.png next to the exe): Kenney's Input Prompts
# (CC0), the "Keyboard & Mouse/Default" PNGs, kept in the work folder (not in any repository). Without them the prompts show key names as text.
set(PKP_KBM_ICONS_DIR "${PKP_ROOT}/work/assets/kenney_input_prompts/Keyboard & Mouse/Default" CACHE PATH "Kenney Input Prompts PNG folder for assets/kbm_icons")
if(EXISTS "${PKP_KBM_ICONS_DIR}")
    # a script copies them: the folder name has an '&', which the build's command lines (cmd) cannot carry
    file(WRITE "${CMAKE_CURRENT_BINARY_DIR}/copy_kbm_icons.cmake"
        "file(GLOB _png \"${PKP_KBM_ICONS_DIR}/*.png\")
file(COPY \${_png} DESTINATION \"\${DST}\")
")
    add_custom_target(pkp_kbm_icons COMMAND ${CMAKE_COMMAND} "-DDST=$<TARGET_FILE_DIR:pkp>/assets/kbm_icons" -P "${CMAKE_CURRENT_BINARY_DIR}/copy_kbm_icons.cmake" VERBATIM)
    add_dependencies(pkp pkp_kbm_icons)
endif()

file(GLOB PKP_A2B_DLLS "${NXRT_A2B_DIR}/astcdec-*-shared.dll")
foreach(dll ${PKP_A2B_DLLS})
    add_custom_command(TARGET pkp POST_BUILD COMMAND ${CMAKE_COMMAND} -E copy_if_different "${dll}" "$<TARGET_FILE_DIR:pkp>")
endforeach()
