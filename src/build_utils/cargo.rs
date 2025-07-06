pub fn warn(msg: impl AsRef<str>) {
    println!("cargo::warning={}", msg.as_ref());
}

pub fn error(msg: impl AsRef<str>) {
    println!("cargo::error={}", msg.as_ref());
}
