# SPDX-License-Identifier: GPL-3.0-or-later
# The Pokémon Pokopia game module (docs/GAME_BOUNDARY.md, pkp_game.h). Libraries, each a superset of the one before:
#   nxrt_game_pkp_data   identity, branding, factory controls.ini (pure data; the launcher links it)
#   nxrt_game_pkp_core   + title facts, test probes (links nxrt)
#   nxrt_game_pkp        + play-mode hooks (links nxrt_gfx; only with NXRT_BUILD_GFX). pkp.exe links this one.
set(_pkp "${CMAKE_CURRENT_SOURCE_DIR}")
set(_rt "${PROJECT_SOURCE_DIR}")
set(NXRT_GAME_PKP_INCLUDES "${_rt}/include" "${_rt}/games" "${_pkp}" "${_rt}/src/gfx" "${_rt}/src/gfx/play" "${_rt}/src/testtools")

add_library(nxrt_game_pkp_data STATIC "${_pkp}/pkp_data.cpp")
target_include_directories(nxrt_game_pkp_data PUBLIC ${NXRT_GAME_PKP_INCLUDES})
target_compile_features(nxrt_game_pkp_data PUBLIC cxx_std_20)

add_library(nxrt_game_pkp_core STATIC "${_pkp}/pkp_core.cpp" "${_pkp}/pkp_tt_probe.cpp")
target_link_libraries(nxrt_game_pkp_core PUBLIC nxrt_game_pkp_data nxrt)

if(TARGET nxrt_gfx)
    add_library(nxrt_game_pkp STATIC "${_pkp}/pkp_game.cpp" "${_pkp}/pkp_play.cpp" "${_pkp}/pkp_gfx_heap.cpp" "${_pkp}/pkp_kbm.cpp" "${_pkp}/pkp_linkplay.cpp" "${_pkp}/pkp_menu.cpp" "${_pkp}/pkp_movie_test.cpp" "${_pkp}/pkp_quality.cpp" "${_pkp}/pkp_taa.cpp")
    target_link_libraries(nxrt_game_pkp PUBLIC nxrt_game_pkp_core nxrt_gfx)
endif()
