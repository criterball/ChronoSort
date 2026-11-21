# Model sources (verified 2026-11-20)

How each entry was checked:
- **Size and SHA-256:** from the Hugging Face API `GET /api/models/<repo>?blobs=true` (`siblings[].size`, `siblings[].lfs.sha256`). These numbers come from the publisher's repository itself.
- **Licence:** from the model card (`cardData.license`) or the repo README "License" section.
- **Pinning:** URLs use `/resolve/<commit sha>/`, so the bytes cannot change under us.
- **Real-world test:** `CS_NET_TESTS=1 cargo test -p cs-ai real_download` downloads YuNet, resumes from a half file, and checks that a corrupted part file is caught and deleted.

| Pack | Repo @ commit | Files | Licence |
|---|---|---|---|
| faces-opencv | opencv/face_detection_yunet @ 3cc26e7f… | face_detection_yunet_2023mar.onnx 232,589 B | **MIT** (README; research/02 said Apache-2.0, which was wrong for YuNet) |
| | opencv/face_recognition_sface @ 3d708243… | face_recognition_sface_2021dec.onnx 38,696,353 B | Apache-2.0 (README) |
| qwen3-vl-2b | Qwen/Qwen3-VL-2B-Instruct-GGUF @ 52d6c8ff… | Qwen3VL-2B-Instruct-Q4_K_M.gguf 1,107,409,952 B; mmproj-Qwen3VL-2B-Instruct-F16.gguf 819,394,848 B | apache-2.0 |
| qwen3-vl-4b | Qwen/Qwen3-VL-4B-Instruct-GGUF @ 1cd86afb… | Q4_K_M 2,497,281,664 B; mmproj F16 836,180,256 B | apache-2.0 |
| qwen3-vl-8b | Qwen/Qwen3-VL-8B-Instruct-GGUF @ f982a075… | Q4_K_M 5,027,784,800 B; mmproj F16 1,159,029,824 B | apache-2.0 |
| qwen3.5-9b | lmstudio-community/Qwen3.5-9B-GGUF @ 1379f25c… | Qwen3.5-9B-Q4_K_M.gguf 5,627,044,256 B; mmproj-Qwen3.5-9B-BF16.gguf 921,704,480 B | apache-2.0 (base Qwen/Qwen3.5-9B apache-2.0) |

The full hashes are in `crates/cs-ai/catalogue.json`.

## Local files on the dev PC
- `001_…/models/Qwen3.5-9B-Q4_K_M.gguf`: SHA-256 cd76ec20…, which **matches** lmstudio-community exactly. Linking it gives "✓ checked".
- `001_…/models/mmproj-BF16.gguf` (675,569,344 B) matches **neither** lmstudio-community (921,704,480) nor unsloth (921,705,024). Origin unknown, and it may belong to a different Qwen3.5 size. Linking it gives "your own copy". For Qwen3.5 9B, use the verified mmproj download.

## Not included, and why
- InsightFace (buffalo_l etc.): non-commercial licence.
- Gemma / Llama vision models: custom licences with use restrictions (GPL freedom 0).
- F16/Q8 quants: larger for little gain in this use. Could be added later as options.
