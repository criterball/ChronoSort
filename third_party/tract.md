# tract 0.23.10 (tract-onnx, tract-core, tract-linalg)

Reviewed 2026-11-20. Used in `crates/cs-faces` to run the YuNet face detector and the SFace recogniser (ONNX).

## Why tract and not ONNX Runtime (`ort`)
- Pure Rust (with SIMD kernels in tract-linalg): no native DLL to download, vet or ship, and it builds with our windows-gnu toolchain. Official onnxruntime binaries are MSVC builds.
- No telemetry at all. Official ONNX Runtime Windows builds include ETW telemetry hooks that have to be switched off.
- These models are small: about 72 ms to detect per photo and 43 ms to fingerprint a face on the dev PC (one thread each; folders are processed in parallel with rayon).
- ONNX Runtime with DirectML/CUDA can come back later for GPU speed, as an optional tool download vetted under the privacy rule.

## Licence
MIT OR Apache-2.0 (Sonos). GPL-3.0-compatible.

## Health / vulnerabilities
- `cargo audit` (local DB, 2026-11-20): no advisories for tract, ndarray or prost.
- Actively maintained (frequent releases, used in production by Sonos).

## Risk notes
- The model file is a parser input (protobuf). We only load model files whose SHA-256 matches the catalogue (downloaded) or that the user picked themselves. A hostile model is out of scope, but the loader runs in-process. If model import becomes more open, move inference out of process (ARCHITECTURE §8).
- tract-onnx itself has very little `unsafe` (10 occurrences, in tensor plumbing). The SIMD kernels in tract-linalg are unsafe by nature; that part is mature.
- The input size is fixed at load (`with_input_fact`): 1×3×640×640 for detection and 1×3×112×112 for recognition, so photo size never changes the graph.

## Our checks
- Output layout check at every detection: the 12 YuNet outputs must have 6400/1600/400 cells, or we fail loudly.
- Accuracy is checked against public-domain portraits (`scripts/fetch_face_testdata.py`, `cargo run -p cs-faces --example eval`).
