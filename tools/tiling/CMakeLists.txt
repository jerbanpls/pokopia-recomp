# tegra_tiling: Tegra block-linear tile/untile library and CLI. clang-cl + Ninja (scripts/build.ps1).
cmake_minimum_required(VERSION 3.20)
# Static MSVC runtime (/MT): players need no Visual C++ Redistributable. Set before project() so every target below, third-party code
# included, uses it; the libraries this one links must agree (a mismatch is a link error).
if(NOT DEFINED CMAKE_MSVC_RUNTIME_LIBRARY)
    set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
endif()
project(tegra_tiling CXX)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
add_library(tegra_tiling STATIC src/tegra_tiling.cpp)
target_include_directories(tegra_tiling PUBLIC include)
add_executable(tegra_tile cli/tegra_tile.cpp)
target_link_libraries(tegra_tile PRIVATE tegra_tiling)
foreach(t tegra_tiling tegra_tile)
    target_compile_definitions(${t} PRIVATE _CRT_SECURE_NO_WARNINGS)
endforeach()
