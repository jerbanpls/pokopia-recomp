# Make a sibling git worktree of this checkout on a new branch and configure its build (graphics shell off: fast, no network).
#   pwsh scripts\wt_setup.ps1 svc-time          -> ..\svc-time on branch svc-time, build dir build\release configured
# The recompiler ABI header is read from the recomp include folder two levels up (ABI 3).
param([Parameter(Mandatory = $true)][string]$Name, [string]$AbiInclude = "")
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$parent = Split-Path -Parent $root
if (-not $AbiInclude) { $AbiInclude = Join-Path (Split-Path -Parent $parent) "recomp\include" }
$dir = Join-Path $parent $Name
git -C $root worktree add -b $Name $dir HEAD
$bd = Join-Path $dir "build\release"
$cfg = "cmake -S `"$dir`" -B `"$bd`" -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_COMPILER=clang-cl -DCMAKE_CXX_COMPILER=clang-cl -DCMAKE_C_COMPILER_LAUNCHER=sccache -DCMAKE_CXX_COMPILER_LAUNCHER=sccache -DNXRT_BUILD_GFX=OFF -DNXRT_RECOMP_INCLUDE=`"$AbiInclude`""
cmd /c "call `"$(Join-Path $dir 'scripts\env.cmd')`" && $cfg"
exit $LASTEXITCODE
