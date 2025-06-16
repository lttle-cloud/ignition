use std::sync::Arc;

use controller::Controller;
use tonic::{Request, Response, Status};
use util::result::Result;

use crate::ignition_proto::image::{PullImageRequest, PullImageResponse};
use crate::ignition_proto::image_server::Image;

pub struct ImageApiConfig {}

pub struct ImageApi {
    config: ImageApiConfig,
    controller: Arc<Controller>,
}

impl ImageApi {
    pub fn new(controller: Arc<Controller>, config: ImageApiConfig) -> Result<Self> {
        Ok(Self { controller, config })
    }
}

#[tonic::async_trait]
impl Image for ImageApi {
    async fn pull(
        &self,
        request: Request<PullImageRequest>,
    ) -> Result<Response<PullImageResponse>, Status> {
        let request = request.into_inner();

        let image_name = request.image;

        let (image, did_pull) = self
            .controller
            .pull_image_if_needed(&image_name)
            .await
            .map_err(|e| Status::internal(format!("failed to pull image: {}", e)))?;

        Ok(Response::new(PullImageResponse {
            image_name: image.reference,
            digest: image.digest,
            was_downloaded: did_pull,
        }))
    }
}
