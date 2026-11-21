//! The whole pipeline against the synthetic corpus: every file must land
//! exactly where `manifest.json` says.

use cs_core::{DatePolicy, analyze};
use cs_model::civil::unix;

#[test]
fn corpus_matches_manifest() {
    let d = tempfile::tempdir().unwrap();
    let manifest = cs_testgen::generate(d.path()).unwrap();
    let root = d.path().join(cs_testgen::SOURCE_DIR);
    let before = cs_core::hash_file(&root.join("Camera/DSC_0001.jpg")).unwrap();

    let policy = DatePolicy::new(unix(2026, 11, 20, 0, 0, 0));
    let a = analyze(&root, &policy).unwrap();
    assert!(a.errors.is_empty(), "scan errors: {:?}", a.errors);

    let r = cs_testgen::eval::evaluate(&manifest, &a.items);
    for (p, want, got) in &r.failures {
        eprintln!("FAIL {p}\n     expected: {want}\n     got:      {got}");
    }
    assert!(r.ok(), "{} of {} passed", r.passed, r.checked);

    // Analysis is read-only.
    assert_eq!(cs_core::hash_file(&root.join("Camera/DSC_0001.jpg")).unwrap(), before);
}
