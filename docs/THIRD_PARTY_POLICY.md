# Third-party code policy

Every external project we reuse (as a dependency, a bundled tool, a model, or code we learned from or adapted) gets a review note: `third_party/<name>.md`.

## Review checklist
1. **License:** must be compatible with GPL-3.0-or-later.
   - MIT, BSD, Apache-2.0, LGPL, GPL-2.0-or-later and GPL-3 are OK.
   - GPL-2.0-only is **not** compatible.
   - AGPL code: don't copy it. Study the design only.
   - Non-commercial or research-only licenses (e.g. InsightFace models): never bundled, because they conflict with GPL freedom 0.
2. **Health:** last release date, open issues and maintainer activity.
3. **Known vulnerabilities:** RustSec (`cargo audit`), the GitHub security advisories of the project, CVE search, and its issue tracker (search crash/overflow/security).
4. **Code read of the parts we use.** Look for:
   - unchecked lengths, offsets and integer overflow in parsers
   - `unsafe` blocks in Rust and their invariants
   - path traversal (`..` and absolute paths inside archives or sidecars)
   - symlink and junction following
   - TOCTOU races (check-then-use on files)
   - time-zone and date edge cases (DST, leap seconds, year 2038, pre-1970)
   - error paths that leave partial files behind
5. **Fuzzing:** any parser that reads file contents gets a cargo-fuzz target, run before each release.
6. **Improvements:** write down faster or better approaches we found, and send fixes upstream where possible (pay it forward).
7. **Isolation:** C/C++ or Perl parsers run out of process (see ARCHITECTURE §8).

## Note template
```
# <name> <version>
Source / license / how we use it
Findings: bugs | security | performance
What we changed or did differently
Upstream issues/PRs filed
```

What we can promise honestly: careful review, fuzzing and isolation make problems far less likely and limit the damage. They can't guarantee there are no unknown (zero-day) bugs. That's why the design assumes a parser can fail.
