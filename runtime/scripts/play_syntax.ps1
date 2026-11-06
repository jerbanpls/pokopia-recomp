# Syntax-check (no code generation, no link) one or more sources of the runtime with the include paths of the play worktree.
#   pwsh scripts\play_syntax.ps1 src\services\hid.cpp [more.cpp ...]
# Exit code 0 = every file compiles. Uses the SDL / ImGui / Vulkan headers of game-build\build\play (override with -Build).
param([Parameter(Mandatory, Position = 0, ValueFromRemainingArguments)][string[]]$Files, [string]$Build = "")
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "proj.ps1")
if (-not $Build) { $Build = Join-Path $proj "game-build\build\play" }
$inc = "/I`"$root\include`" /I`"$root\src\gfx`" /I`"$root\src\gfx\nvn`" /I`"$root\src\gfx\nvn\third_party`" /I`"$root\src\services`" /I`"$root\src\testtools`" " +
       "/I`"$Build\_deps\sdl3-src\include`" /I`"$Build\_deps\imgui-src`" /I`"$Build\_deps\vulkan_headers-src\include`" /I`"$Build\_deps\volk-src`" /I`"$recomp`""
$env_cmd = Join-Path $PSScriptRoot "env.cmd"
$rc = 0
foreach ($f in $Files) {
    $p = if ([System.IO.Path]::IsPathRooted($f)) { $f } else { Join-Path $root $f }
    cmd /c "call `"$env_cmd`" >nul 2>&1 && clang-cl /nologo /std:c++20 /Zs /EHsc /W3 -Wno-unused-parameter -Wno-missing-field-initializers -Wno-cast-function-type-mismatch -Wno-cast-function-type-strict -Wno-language-extension-token /DNOMINMAX /DWIN32_LEAN_AND_MEAN /D_CRT_SECURE_NO_WARNINGS /D_WIN32_WINNT=0x0A00 $inc `"$p`""
    if ($LASTEXITCODE -ne 0) { $rc = 1 } else { Write-Host "OK $f" }
}
exit $rc
