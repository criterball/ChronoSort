# ExifTool 13.64

- **Source:** https://exiftool.org (Windows 64-bit zip via SourceForge). SHA-256 `72c29c44…804af` matches the official checksums.txt.
- **Installed:** `D:\Claude_Code_Home\tools\exiftool\exiftool.exe` (+ `exiftool_files\` with bundled Perl).
- **License:** same as Perl (Artistic or GPL). It runs as a separate process, so it does not affect ChronoSort's license. Can be bundled.
- **How we use it:** a second-opinion reader and the writer for metadata on **copies only**. A long-running `-stay_open` process is planned for Phase 1. The primary reader will be native Rust (faster, memory-safe).

## Findings so far
- Correctly read every case in the synthetic corpus: EXIF, OffsetTime, scanner tags, NEF-as-TIFF, MP4/MOV `mvhd` (UTC), XMP sidecar, PNG-with-.jpg-extension (FileType=PNG), NFC/NFD names (with `-charset filename=utf8`).
- Hostile EXIF (`Damaged/evil_exif.jpg`: count 0xFFFFFFF0, self-referencing IFDs): rejected with
  `Warning: Invalid size (4294967280) for IFD0 tag 0x010f Make`. It did not hang and returned at once. Good.
- Note: without `-charset filename=utf8`, Windows non-ASCII filenames can be misread. Always pass it.
- Note: QuickTime dates are returned as UTC unless `-api QuickTimeUTC` is used. We must handle time zones ourselves (see ARCHITECTURE §4).

## Isolation plan
Runs out of process with a timeout per file and the source opened read-only. Its output is parsed as JSON (`-j`). We never use `-overwrite_original` on source files; the source is never a write target.

## Still to do
- Check the ExifTool changelog for security fixes before upgrading.
- Fuzz our JSON-parsing side with odd tag values.
