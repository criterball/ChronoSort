# kamadak-exif 0.6.2

- **Source:** crates.io `kamadak-exif`. BSD-2-Clause, compatible with GPL-3.0.
- **How we use it:** primary EXIF reader for JPEG, PNG, WebP, HEIF/AVIF and TIFF/RAW (`crates/cs-meta/src/exif.rs`).
- **Size:** about 6.6k lines of pure Rust. One dependency (`mutate_once`).

## Review (2026-11-20)
| Area | Finding |
|---|---|
| `unsafe` | One block (`value.rs`: `&Value` → `&UIntValue`). The target is `#[repr(transparent)]` over `Value` and the variant is checked first. **Sound.** |
| Integer overflow | Value length uses `checked_mul`; offset+length is checked against the buffer before slicing. |
| IFD loops | The next-IFD chain is capped at 8 IFDs (`ifd_num >= 8` → error), and the upstream tests include `inf_loop_by_next` / `inf_loop_by_exif_next`. Sub-IFD pointers are only followed from the TIFF context, so a self-pointing Exif IFD does not recurse. Our hostile corpus file `Damaged/evil_exif.jpg` confirms this: rejected, no hang. |
| Allocation from hostile lengths | `read_exact_len` uses `take(len).read_to_end`, so memory grows only as real bytes arrive, and a fake 4 GB length in a 1 KB file cannot OOM. JPEG segments are ≤ 64 KB by format. HEIF Exif is capped at 128 KB (`MAX_EXIF_SIZE`). |
| **Weakness** | For TIFF-based files, `read_from_container` calls `read_to_end` on the **whole file**, so a multi-GB TIFF/RAW is loaded into RAM. **Our fix:** for `Format::Tiff` we read at most 64 MiB ourselves and call `read_raw` (`TIFF_EXIF_LIMIT`). EXIF lives at the start of RAW files, so nothing is lost in practice. |
| Robustness | 1,020 mutated files (bit flips, truncation, extreme length fields, random data) through the full pipeline (`crates/cs-core/tests/mutation.rs`): no panics. |

## Possible upstream contribution
Offer an optional size limit for TIFF in `read_from_container` (the same issue we work around). Not filed yet.
