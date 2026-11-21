# Source this: . scripts/env.sh   (Git Bash)
_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
_here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export RUSTUP_HOME="$_root/tools/rust/rustup"
export CARGO_HOME="$_root/tools/rust/cargo"
# One consistent MinGW: w64devkit (GCC 16) links Rust and compiles C code.
# Rust's windows-gnu target asks for -lgcc_eh, which w64devkit folds into
# libgcc.a, so an empty libgcc_eh.a shim satisfies the linker.
_shim="$_here/mingw-shim"
if [ ! -f "$_shim/libgcc_eh.a" ]; then
  mkdir -p "$_shim" && "$_root/tools/mingw64/bin/ar.exe" rcs "$_shim/libgcc_eh.a"
fi
export LIBRARY_PATH="$_shim${LIBRARY_PATH:+:$LIBRARY_PATH}"
export PATH="$CARGO_HOME/bin:$_root/tools/mingw64/bin:$_root/tools/node:$PATH"
# keep all temp files inside the sandbox
export TMP="$_root/temp"
export TEMP="$TMP"
unset _root _here _shim
