pub mod device;
pub mod handler;
pub mod overlay_backend;

pub fn get_block_mount_source_by_index(index: u16) -> String {
    // 0 -> /dev/vda, 1 -> /dev/vdb, etc.
    let char = (b'a' + index as u8) as char;
    format!("/dev/vd{}", char)
}
