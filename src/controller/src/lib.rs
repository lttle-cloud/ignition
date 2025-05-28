use image::ImagePool;
use net::{ip::IpPool, tap::TapPool};
use sds::Store;
use volume::VolumePool;

mod image;
mod net;
mod volume;

#[derive(Clone)]
pub struct ControllerConfig {}

#[derive(Clone)]
pub struct Controller {
    config: ControllerConfig,
    store: Store,
    image_pool: ImagePool,
    // vm_pool: VmPool,
    tap_pool: TapPool,
    svc_ip_pool: IpPool,
    vm_ip_pool: IpPool,
    volume_pool: VolumePool,
}

// impl Controller {
//     pub fn new(store: Store) -> Self {
//         Self {}
//     }
// }
