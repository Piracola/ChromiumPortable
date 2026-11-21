pub fn info(msg: impl AsRef<str>) {
    println!("[INFO] {}", msg.as_ref());
}

pub fn warn(msg: impl AsRef<str>) {
    println!("[WARN] {}", msg.as_ref());
}

pub fn ok(msg: impl AsRef<str>) {
    println!("[OK] {}", msg.as_ref());
}

pub fn fail(msg: impl AsRef<str>) {
    eprintln!("[FAIL] {}", msg.as_ref());
}
