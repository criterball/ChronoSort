# FFmpeg 9.1 (essentials build by Gyan Doshi)

Source / license / how we use it
- Download (optional, user-triggered in Settings -> AI models): https://github.com/GyanD/codexffmpeg/releases/download/9.1/ffmpeg-9.1-essentials_build.zip
- 119,467,219 bytes, SHA-256 b9bbbb3acece000cbeec994f856649354e52c773807841e78908374b24d658fd
  (computed locally 2026-11-21; matches the digest GitHub lists for the asset).
- License: GPL-3.0 (this build enables GPL parts). Compatible with ChronoSort (GPL-3.0-or-later).
  Not bundled: the user downloads it directly from the publisher, so we do not redistribute it.
- Use: `ffmpeg.exe` makes one preview frame of a video (about 1 s in) or of a HEIC/AVIF photo,
  written as PNG to stdout (crates/cs-thumb/src/external.rs).

Isolation and hardening (our side)
- Separate process, no shell, arguments as a list; `-nostdin`; `-protocol_whitelist file,pipe` and the
  input passed as `file:<path>`, so a crafted file cannot make FFmpeg open network or other protocols.
- 20 s time limit (then killed); output capped at 64 MiB; PNG decoded with ChronoSort's pixel limits.
- No console window (CREATE_NO_WINDOW). Errors fall back to the embedded thumbnail or a placeholder.

Findings
- Not yet done: CVE / advisory review for 9.1, and a real run on sample files (HEIC tile grids,
  rotation, short clips). Running the downloaded binary needs the user's OK in the dev environment.

What we changed or did differently
- Nothing changed in FFmpeg. Large RAW previews are extracted without FFmpeg (cs-meta preview.rs).
