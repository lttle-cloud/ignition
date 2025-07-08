use std::{
    env,
    path::{Path, PathBuf},
};

pub fn warn(msg: impl AsRef<str>) {
    println!("cargo::warning={}", msg.as_ref());
}

pub fn error(msg: impl AsRef<str>) {
    println!("cargo::error={}", msg.as_ref());
}

pub fn out_dir_path(rel: impl AsRef<str>) -> PathBuf {
    let out_dir = env::var("OUT_DIR").unwrap();
    let path = Path::new(&out_dir).join(rel.as_ref());

    warn(format!("out_dir_path {}", path.display()));

    path
}
