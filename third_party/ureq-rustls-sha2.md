# ureq 3.5.2 + rustls 0.23.46 (ring 0.17.14, webpki-roots 1.0.12) + sha2 0.10.9

Reviewed 2026-11-20. Used only in `crates/cs-ai/src/download.rs`, which is the only network code in ChronoSort: it downloads model files when the user clicks Download.

## Licences
- ureq: MIT OR Apache-2.0. rustls: Apache-2.0 OR ISC OR MIT. ring: Apache-2.0 AND ISC. sha2: MIT OR Apache-2.0. All GPL-3.0-compatible.
- webpki-roots: CDLA-Permissive-2.0, a permissive data licence covering the Mozilla CA list. Include its notice in the release THIRD_PARTY notices.

## Health / vulnerabilities
- `cargo audit` (local DB, 2026-11-20): no advisories for ureq, rustls, ring, webpki-roots, sha2, flate2.
- All are widely used and actively maintained.

## How we use it (and limits we put on it)
- `max_redirects(0)`: redirects are followed by hand, at most 5 hops. Every hop must be HTTPS to `huggingface.co`, `*.huggingface.co` or `*.hf.co`. Catalogue URLs must be `https://huggingface.co/...` and pinned to a 40-hex commit (unit-tested).
- `https_only(true)`. URLs with user-info (`@`) or explicit ports are refused.
- Request = GET + generic User-Agent `ChronoSort/<ver> (model download)` + optional `Range`. No cookies, no tokens, no user or photo data.
- The response is written to `<file>.part` and never past the expected size. The final file must match the exact byte size and SHA-256 from the catalogue, otherwise the part file is deleted. A 206 resume must start exactly at the part length; if the server ignores Range (200), the download starts over.
- Body timeouts: connect 20 s, response headers 60 s. The body has no total timeout (multi-GB files); Cancel is checked every 1 MiB.

## Findings / ideas
- webpki-roots bundles a CA list, which goes stale over time. Option: `rustls-platform-verifier` to use the OS certificate store (Windows/macOS keep it updated). Revisit before the first release.
- sha2 picks SHA-NI / AVX2 at runtime through `cpufeatures`. Hashing the 5.6 GB Qwen3.5 file is disk-bound.
- A future option is a per-read idle timeout, so a stalled connection fails by itself instead of needing Cancel.
