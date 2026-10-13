//! Log prefix helpers. The prefixes are a CI-facing contract (migration doc S1):
//! `[INFO] / [WARN] / [OK] / [DONE]` go to stdout, `[FAIL]` goes to stderr.
//! Keep byte-identical with the Python engine so Actions logs stay diffable.

pub fn info(msg: impl AsRef<str>) {
    println!("[INFO] {}", msg.as_ref());
}

pub fn warn(msg: impl AsRef<str>) {
    println!("[WARN] {}", msg.as_ref());
}

pub fn ok(msg: impl AsRef<str>) {
    println!("[OK] {}", msg.as_ref());
}

pub fn done(msg: impl AsRef<str>) {
    println!("[DONE] {}", msg.as_ref());
}

pub fn fail(msg: impl AsRef<str>) {
    eprintln!("[FAIL] {}", msg.as_ref());
}
