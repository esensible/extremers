//! `LANG_SPEC`: the language spec's draft ("draft 4"), from the first line
//! of lang/SPEC.md (`# … (draft 4)`), for `version()`.

fn main() {
    let spec = "../../lang/SPEC.md";
    println!("cargo:rerun-if-changed={spec}");
    let text = std::fs::read_to_string(spec).unwrap_or_else(|e| panic!("{spec}: {e}"));
    let first = text.lines().next().unwrap_or_default();
    let draft = first
        .rsplit_once('(')
        .and_then(|(_, rest)| rest.strip_suffix(')'))
        .filter(|d| d.starts_with("draft "))
        .unwrap_or_else(|| panic!("{spec}: its first line names no draft: {first:?}"));
    println!("cargo:rustc-env=LANG_SPEC={draft}");
}
