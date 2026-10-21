//! `openapi.json` is generated from the API routes. This fails when the
//! committed copy is out of date; `mise run openapi` rewrites it.

use std::path::Path;

#[test]
fn openapi_json_is_current() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("openapi.json");
    let generated = credits::web::openapi()
        .to_pretty_json()
        .expect("the spec serializes")
        + "\n";
    if std::env::var_os("UPDATE_OPENAPI").is_some() {
        std::fs::write(&path, &generated).expect("openapi.json is writable");
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "openapi.json is out of date with the API routes; run `mise run openapi` and commit it"
    );
}
