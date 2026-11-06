# SPDX-License-Identifier: GPL-3.0-or-later
# nxrecomp: the AArch64 -> C++ recompiler. This file exposes the build function for the game.
#
#   cmake --preset default                 (from a shell with the MSVC environment: scripts\nx_env.cmd cmake --preset default)
#   cmake --build --preset default
#
# A game build includes cmake/NxRecomp.cmake and calls nxrecomp_add_module(...) for each module, then links the runtime; see
# docs in cmake/NxRecomp.cmake.
cmake_minimum_required(VERSION 3.25)
project(nxrecomp C CXX)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
include(cmake/NxRecomp.cmake)
add_library(nxrecomp_headers INTERFACE)
target_include_directories(nxrecomp_headers INTERFACE "${CMAKE_CURRENT_SOURCE_DIR}/include")
