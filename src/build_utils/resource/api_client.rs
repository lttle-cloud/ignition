use anyhow::Result;
use damascus::{
    aat::AAT,
    generate::typescript::TypeScriptGenerator,
    header_value, path,
    spec::{PathSegment, Spec, Type, Upgrade},
    type_of,
};

use crate::{
    build_utils::{cargo, fs::write_if_changed},
    resources::{
        ResourceBuildInfo,
        core::{
            AllocatedBuilder, CLIENT_COMPAT_VERSION, DeleteNamespaceParams,
            DeleteNamespaceResponse, ExecParams, ListNamespaces, LogStreamItem, LogStreamParams,
            Me, QueryParams, QueryResponse, RegistryRobot,
        },
        gadget::{GadgetInitRunParams, GadgetInitRunResponse},
    },
};

pub async fn build_api_spec(resources: &[ResourceBuildInfo]) -> Result<Spec> {
    let mut spec = Spec::new("ignition")
        .description("Ignition is a platform for building and deploying machines.")
        .organization("ignition")
        .website("https://ignition.com")
        .docs("https://docs.ignition.com")
        .repository("https://github.com/ignition/ignition")
        .header("x-ignition-compat", header_value!(CLIENT_COMPAT_VERSION))
        .header("x-ignition-token", header_value!(apiToken: String));

    spec = core_api_spec(spec);

    for resource in resources {
        spec = resource_api_spec(spec, resource);
    }

    let debug_out_path = cargo::build_out_dir_path("api_spec_debug.txt");
    write_if_changed(debug_out_path, format!("{:#?}", spec)).await?;

    Ok(spec)
}

fn core_api_spec(spec: Spec) -> Spec {
    spec.service("auth", |service| {
        service
            .get("me", path!("core", "me"), |endpoint| {
                endpoint.response(type_of!(Me))
            })
            .get(
                "registry_robot_auth",
                path!("core", "registry", "robot"),
                |endpoint| endpoint.response(type_of!(RegistryRobot)),
            )
            .get(
                "builder_registry_robot_auth",
                path!("core", "registry", "builder-robot"),
                |endpoint| endpoint.response(type_of!(RegistryRobot)),
            )
    })
    .service("namespace", |service| {
        service
            .get("list", path!("core", "namespaces"), |endpoint| {
                endpoint.response(type_of!(ListNamespaces))
            })
            .put(
                "delete",
                path!("core", "namespaces", "delete"),
                |endpoint| {
                    endpoint
                        .body(type_of!(DeleteNamespaceParams))
                        .response(type_of!(DeleteNamespaceResponse))
                },
            )
    })
    .service("machine", |service| {
        service
            .get("logs", path!("core", "logs"), |endpoint| {
                endpoint
                    .header("x-ignition-namespace", header_value!(namespace: String))
                    .upgrade(Upgrade::Ws)
                    .query(type_of!(LogStreamParams))
                    .response(type_of!(LogStreamItem).wrap_stream())
            })
            .get("exec", path!("core", "exec"), |endpoint| {
                endpoint
                    .header("x-ignition-namespace", header_value!(namespace: String))
                    .upgrade(Upgrade::Ws)
                    .query(type_of!(ExecParams))
                    .response(Type::void().wrap_stream())
            })
    })
    .service("runtime", |service| {
        service.put("query", path!("core", "query"), |endpoint| {
            endpoint
                .body(type_of!(QueryParams))
                .response(type_of!(QueryResponse))
        })
    })
    .service("build", |service| {
        service.put(
            "alloc_builder",
            path!("core", "build", "alloc"),
            |endpoint| endpoint.response(type_of!(AllocatedBuilder)),
        )
    })
    .service("gadget", |service| {
        service.put("init", path!("gadget", "run", "init"), |endpoint| {
            endpoint
                .body(type_of!(GadgetInitRunParams))
                .response(type_of!(GadgetInitRunResponse))
        })
    })
}

fn resource_api_spec(spec: Spec, resource: &ResourceBuildInfo) -> Spec {
    if !resource.configuration.generate_service {
        return spec;
    }

    let latest_version = resource.versions.iter().find(|v| v.latest).expect(&format!(
        "No latest version found for resource {}",
        resource.name
    ));

    let latest_version_schema = resource
        .d_version_schemas
        .get(latest_version.variant_name)
        .expect(&format!(
            "No schema found for latest version {}",
            latest_version.variant_name
        ));

    let latest_version_type = Type::Schema(latest_version_schema.clone());
    let status_type = Type::Schema(resource.d_status_schema.clone());

    let latest_and_status_tuple_type =
        Type::Tuple(vec![latest_version_type.clone(), status_type.clone()]);

    // panic!("{:#?}", resource);

    spec.service(resource.tag, |service| {
        let mut service = service;

        if resource.configuration.generate_service_get {
            service = service.get(
                "get",
                vec![
                    PathSegment::Literal(resource.tag.to_string()),
                    PathSegment::Type {
                        name: "name".to_string(),
                        r#type: type_of!(String),
                    },
                ],
                |endpoint| {
                    endpoint
                        .header("x-ignition-namespace", header_value!(namespace: String))
                        .response(latest_and_status_tuple_type.clone())
                },
            );
        }

        if resource.configuration.generate_service_list {
            service = service.get(
                "list",
                vec![PathSegment::Literal(resource.tag.to_string())],
                |endpoint| {
                    endpoint
                        .header(
                            "x-ignition-namespace",
                            header_value!(namespace: Option<String>),
                        )
                        .response(latest_and_status_tuple_type.wrap_list())
                },
            )
        }

        if resource.configuration.generate_service_get_status {
            service = service.get(
                "status",
                vec![
                    PathSegment::Literal(resource.tag.to_string()),
                    PathSegment::Type {
                        name: "name".to_string(),
                        r#type: type_of!(String),
                    },
                    PathSegment::Literal("status".to_string()),
                ],
                |endpoint| {
                    endpoint
                        .header("x-ignition-namespace", header_value!(namespace: String))
                        .response(status_type.clone())
                },
            )
        }

        if resource.configuration.generate_service_delete {
            service = service.delete(
                "delete",
                vec![
                    PathSegment::Literal(resource.tag.to_string()),
                    PathSegment::Type {
                        name: "name".to_string(),
                        r#type: type_of!(String),
                    },
                ],
                |endpoint| {
                    endpoint.header("x-ignition-namespace", header_value!(namespace: String))
                },
            )
        }

        if resource.configuration.generate_service_set {
            service = service.put(
                "apply",
                vec![PathSegment::Literal(resource.tag.to_string())],
                |endpoint| endpoint.body(Type::Schema(resource.d_root_type_schema.clone())),
            )
        }

        service
    })
}

pub async fn build_ts_client(spec: &Spec) -> Result<()> {
    let aat_debug_path = cargo::build_out_dir_path("aat_debug.txt");

    let ts_client_out_path =
        cargo::workspace_root_dir_path("sdk/typescript-client/src/client.ts").await?;

    let aat = AAT::from_spec(spec)?;
    aat.validate()?;
    write_if_changed(aat_debug_path, format!("{:#?}", aat)).await?;

    let code = TypeScriptGenerator::generate(&aat)?;
    write_if_changed(ts_client_out_path, code).await?;

    Ok(())
}
