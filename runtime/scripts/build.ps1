# Configure + build the runtime. Run from anywhere:   pwsh scripts\build.ps1 [-Config Release] [-Clean]
# Works relative to the checkout the script lives in, so git worktrees build on their own. Output: <checkout>\build\<config>
param([string]$Config = "Release", [switch]$Clean, [switch]$Verbose)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$bd = Join-Path $root ("build\" + $Config.ToLower())
if ($Clean -and (Test-Path $bd)) { Remove-Item -Recurse -Force $bd }
$env_cmd = Join-Path $PSScriptRoot "env.cmd"
$cfg = "cmake -S `"$root`" -B `"$bd`" -G Ninja -DCMAKE_BUILD_TYPE=$Config -DCMAKE_C_COMPILER=clang-cl -DCMAKE_CXX_COMPILER=clang-cl -DCMAKE_C_COMPILER_LAUNCHER=sccache -DCMAKE_CXX_COMPILER_LAUNCHER=sccache"
$bld = "cmake --build `"$bd`""
if ($Verbose) { $bld += " --verbose" }
$cmd = "call `"$env_cmd`" && $cfg && $bld"
cmd /c $cmd
exit $LASTEXITCODE
