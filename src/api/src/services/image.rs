use sds::{Collection, Store};
use tonic::{Request, Response, Status};
use util::result::Result;

use crate::data::image::{
    image_pull_secret_collection_prefix, image_pull_secret_key, ImagePullSecret,
};
use crate::ignition_proto::image::{
    self, CreatePullSecretRequest, CreatePullSecretResponse, DeletePullSecretRequest,
    ImageUploadRequest, ImageUploadResponse, ImportImageRequest, ImportImageResponse,
    ListImagesResponse, ListPullSecretsResponse,
};
use crate::ignition_proto::image_server::Image;
use crate::ignition_proto::util::Empty;
use crate::services::auth::get_authenticated_user;

pub struct ImageService {
    store: Store,
    image_pull_secret_collection: Collection<ImagePullSecret>,
}

impl ImageService {
    pub fn new(store: Store) -> Result<Self> {
        let image_pull_secret_collection =
            store.collection::<ImagePullSecret>("image_pull_secret")?;

        Ok(Self {
            store,
            image_pull_secret_collection,
        })
    }
}

#[tonic::async_trait]
impl Image for ImageService {
    async fn create_pull_secret(
        &self,
        request: Request<CreatePullSecretRequest>,
    ) -> Result<Response<CreatePullSecretResponse>, Status> {
        let user = get_authenticated_user(&request)?;
        let inner_request = request.get_ref();

        let mut tx = self
            .store
            .write_txn()
            .map_err(|_| Status::internal("Failed to create read txn"))?;

        let key = image_pull_secret_key(&user.id, &inner_request.name);
        tx.put(
            &self.image_pull_secret_collection,
            &key,
            &ImagePullSecret {
                name: inner_request.name.clone(),
                username: inner_request.username.clone(),
                password: inner_request.password.clone(),
                owner_id: user.id.clone(),
            },
        )
        .map_err(|_| Status::internal("Failed to create pull secret"))?;

        tx.commit()
            .map_err(|_| Status::internal("Failed to commit txn"))?;

        Ok(Response::new(CreatePullSecretResponse {
            name: inner_request.name.clone(),
        }))
    }

    async fn delete_pull_secret(
        &self,
        request: Request<DeletePullSecretRequest>,
    ) -> Result<Response<Empty>, Status> {
        let user = get_authenticated_user(&request)?;
        let inner_request = request.get_ref();

        let mut tx = self
            .store
            .write_txn()
            .map_err(|_| Status::internal("Failed to create read txn"))?;

        tx.del(
            &self.image_pull_secret_collection,
            &image_pull_secret_key(&user.id, &inner_request.name),
        )
        .map_err(|_| Status::internal("Failed to delete pull secret"))?;

        tx.commit()
            .map_err(|_| Status::internal("Failed to commit txn"))?;

        Ok(Response::new(Empty {}))
    }

    async fn list_pull_secrets(
        &self,
        request: Request<Empty>,
    ) -> Result<Response<ListPullSecretsResponse>, Status> {
        let user = get_authenticated_user(&request)?;
        let tx = self
            .store
            .read_txn()
            .map_err(|_| Status::internal("Failed to create read txn"))?;

        let prefix = image_pull_secret_collection_prefix(&user.id);

        let image_pull_secrets = tx
            .prefix_iter(&self.image_pull_secret_collection, &prefix)
            .map_err(|_| Status::internal("Failed to iterate over pull secrets"))?
            .collect::<Result<Vec<_>, sds::Error>>()
            .map_err(|_| Status::internal("failed to collect users"))?
            .into_iter()
            .map(|(_, secret)| {
                let api_secret: image::ImagePullSecret = secret.into();
                api_secret
            })
            .collect::<Vec<_>>();

        Ok(Response::new(ListPullSecretsResponse {
            secrets: image_pull_secrets,
        }))
    }

    async fn upload(
        &self,
        request: Request<tonic::Streaming<ImageUploadRequest>>,
    ) -> Result<Response<ImageUploadResponse>, Status> {
        todo!("Implement upload")
    }

    async fn import(
        &self,
        request: Request<ImportImageRequest>,
    ) -> Result<Response<ImportImageResponse>, Status> {
        todo!("Implement import")
    }

    async fn list_images(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<ListImagesResponse>, Status> {
        todo!("Implement list_images")
    }
}
