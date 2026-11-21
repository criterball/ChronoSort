# Dot-source this: . .\scripts\env.ps1
$root = (Resolve-Path "$PSScriptRoot\..\..").Path
$env:RUSTUP_HOME = "$root\tools\rust\rustup"
$env:CARGO_HOME  = "$root\tools\rust\cargo"
# One consistent MinGW (w64devkit). Empty libgcc_eh.a shim: see env.sh.
$shim = "$PSScriptRoot\mingw-shim"
if (-not (Test-Path "$shim\libgcc_eh.a")) {
  New-Item -ItemType Directory -Force $shim | Out-Null
  & "$root\tools\mingw64\bin\ar.exe" rcs "$shim\libgcc_eh.a"
}
$env:LIBRARY_PATH = $shim
$env:PATH = "$env:CARGO_HOME\bin;$root\tools\mingw64\bin;$root\tools\node;$env:PATH"
$env:TMP = "$root\temp"; $env:TEMP = "$root\temp"
