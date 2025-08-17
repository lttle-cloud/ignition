use anyhow::Result;
use tokio::fs::write;

use crate::{
    build_utils::cargo,
    machinery::api_schema::{
        ApiMethod, ApiPathSegment, ApiRequest, ApiResponse, ApiSchema, ApiService, ApiVerb,
    },
};

pub async fn build_rust_api_client(api_schema: &ApiSchema) -> Result<()> {
    let client_out_path = cargo::build_out_dir_path("api_client.rs");

    let mut src = String::new();
    src.push_str("#[allow(dead_code, unused)]\n");
    src.push_str("pub mod api_client {\n");
    src.push_str("    use anyhow::Result;\n");
    src.push_str("    use serde::{Deserialize, Serialize};\n");
    src.push_str("    use futures_util::{SinkExt, StreamExt};\n");
    src.push_str(
        "    use tokio_tungstenite::{WebSocketStream, tungstenite::Message, MaybeTlsStream};\n",
    );
    src.push_str("    use tokio::net::TcpStream;\n");
    src.push_str("    use std::marker::PhantomData;\n");
    src.push_str("    use serde_urlencoded;\n");
    src.push_str("    use url::Url;\n");
    src.push_str("    use tokio_tungstenite::tungstenite::client::IntoClientRequest;\n");
    src.push_str("    use tungstenite::http::HeaderValue;\n\n");
    src.push_str("    use crate::resources::metadata::Namespace;\n\n");

    // Generate config struct
    src.push_str("    #[derive(Clone)]\n");
    src.push_str("    pub struct ApiClientConfig {\n");
    src.push_str("        pub base_url: String,\n");
    src.push_str("        pub token: String,\n");
    src.push_str("    }\n\n");

    src.push_str("    pub struct IgnitionWsStream<T> {\n");
    src.push_str("        stream: WebSocketStream<MaybeTlsStream<TcpStream>>,\n");
    src.push_str("        _phantom: PhantomData<T>,\n");
    src.push_str("    }\n\n");

    src.push_str("    impl<T> IgnitionWsStream<T> {\n");
    src.push_str(
        "        pub fn new(stream: WebSocketStream<MaybeTlsStream<TcpStream>>) -> Self {\n",
    );
    src.push_str("            Self { \n");
    src.push_str("                stream,\n");
    src.push_str("                _phantom: PhantomData,\n");
    src.push_str("            }\n");
    src.push_str("        }\n\n");

    src.push_str("        pub async fn next(&mut self) -> Option<T>\n");
    src.push_str("        where\n");
    src.push_str("            T: for<'de> serde::Deserialize<'de>,\n");
    src.push_str("        {\n");
    src.push_str("            let message = self.stream.next().await;\n");
    src.push_str("            match message {\n");
    src.push_str("                Some(Ok(Message::Text(text))) => {\n");
    src.push_str("                    match serde_json::from_str(&text) {\n");
    src.push_str("                        Ok(message) => Some(message),\n");
    src.push_str("                        Err(_) => None,\n");
    src.push_str("                    }\n");
    src.push_str("                },\n");
    src.push_str("                _ => None,\n");
    src.push_str("            }\n");
    src.push_str("        }\n\n");
    src.push_str("    }\n\n");

    // Generate main ApiClient struct
    src.push_str("    pub struct ApiClient {\n");
    src.push_str("        config: ApiClientConfig,\n");
    src.push_str("    }\n\n");

    // Generate ApiClient implementation
    src.push_str("    impl ApiClient {\n");
    src.push_str("        pub fn new(config: ApiClientConfig) -> Self {\n");
    src.push_str("            Self { config }\n");
    src.push_str("        }\n\n");

    // Generate service methods
    for service in &api_schema.services {
        let service_client_name = format!("{}ApiClient", service.name);
        src.push_str(&format!(
            "        pub fn {}(&self) -> {} {{\n",
            service.tag, service_client_name
        ));
        src.push_str(&format!("            {} {{\n", service_client_name));
        src.push_str("                config: self.config.clone(),\n");
        src.push_str("            }\n");
        src.push_str("        }\n\n");
    }

    src.push_str("    }\n\n");

    // Generate individual service clients
    for service in &api_schema.services {
        let service_client_name = format!("{}ApiClient", service.name);

        src.push_str(&format!("    pub struct {} {{\n", service_client_name));
        src.push_str("        config: ApiClientConfig,\n");
        src.push_str("    }\n\n");

        src.push_str(&format!("    impl {} {{\n", service_client_name));

        // Generate methods for each service
        for method in &service.methods {
            if method.verb == ApiVerb::WebSocket {
                generate_websocket_method(&mut src, service, method);
                continue;
            }

            generate_method(&mut src, service, method);
        }

        src.push_str("    }\n\n");
    }

    src.push_str("}\n\n");

    write(&client_out_path, src).await?;

    Ok(())
}

fn generate_url_format_impl(methods: &ApiMethod) -> String {
    let mut url_parts = Vec::new();
    let mut args = Vec::new();
    for segment in &methods.path {
        match segment {
            ApiPathSegment::Static { value } => {
                url_parts.push(value.to_string());
            }
            ApiPathSegment::ResourceName => {
                url_parts.push("{}".to_string());
                args.push("name.as_ref()".to_string());
            }
        }
    }

    if url_parts.len() == 1 {
        format!(
            "let url = format!(\"{{}}/{}\", self.config.base_url);",
            url_parts[0]
        )
    } else {
        format!(
            "let url = format!(\"{{}}/{}\", self.config.base_url, {});",
            url_parts.join("/"),
            args.join(", ")
        )
    }
}

fn generate_response_inner_type(service: &ApiService, response: &Option<ApiResponse>) -> String {
    match &response {
        Some(ApiResponse::SchemaDefinition { list, name, .. }) => {
            if *list {
                format!("Vec<crate::{}::{}>", service.crate_path, name)
            } else {
                format!("crate::{}::{}", service.crate_path, name)
            }
        }
        Some(ApiResponse::TupleSchemaDefinition { list, names, .. }) => {
            let full_names = names
                .iter()
                .map(|name| format!("crate::{}::{}", service.crate_path, name))
                .collect::<Vec<_>>();

            if *list {
                format!("Vec<({})>", full_names.join(", "))
            } else {
                format!("({})", full_names.join(", "))
            }
        }
        None => "()".to_string(),
    }
}

fn generate_method(src: &mut String, service: &ApiService, method: &ApiMethod) {
    let method_name = &method.name;
    let verb = match method.verb {
        ApiVerb::Get => "get",
        ApiVerb::Put => "put",
        ApiVerb::Delete => "delete",
        ApiVerb::WebSocket => unreachable!(),
    };

    // Build URL path
    let url_construction = generate_url_format_impl(method);

    let namespaced = service.namespaced || method.namespaced;

    let generate_namespace_header =
        namespaced && (method.verb == ApiVerb::Get || method.verb == ApiVerb::Delete);

    // Method signature
    let mut params = Vec::new();
    if generate_namespace_header {
        params.push("namespace: Namespace".to_string());
    }
    if method
        .path
        .iter()
        .any(|s| matches!(s, ApiPathSegment::ResourceName))
    {
        params.push("name: impl AsRef<str>".to_string());
    }
    if let Some(request) = &method.request {
        match request {
            ApiRequest::SchemaDefinition { name } => {
                params.push(format!(
                    "{}: crate::{}::{}",
                    name.to_lowercase(),
                    service.crate_path,
                    name
                ));
            }
            ApiRequest::OptionalSchemaDefinition { name } => {
                params.push(format!(
                    "{}: Option<crate::{}::{}>",
                    name.to_lowercase(),
                    service.crate_path,
                    name
                ));
            }
            ApiRequest::TaggedSchemaDefinition { name, .. } => {
                params.push(format!("{}: {}", name.to_lowercase(), name));
            }
        }
    }

    let return_type = format!(
        "Result<{}>",
        generate_response_inner_type(service, &method.response)
    );

    src.push_str(&format!(
        "        pub async fn {}(&self, {}) -> {} {{\n",
        method_name,
        params.join(", "),
        return_type
    ));

    // Method body
    src.push_str(&format!("            {}\n\n", url_construction));

    src.push_str("            let client = reqwest::Client::new();\n");
    src.push_str(&format!(
        "            let mut request = client.{}(url);\n",
        verb
    ));

    // Add namespace header if namespaced
    if generate_namespace_header {
        src.push_str("            if let Some(namespace) = namespace.as_value() {\n");
        src.push_str(&format!(
            "                request = request.header(\"x-ignition-namespace\", namespace);\n"
        ));
        src.push_str("            }\n");
    }

    // Add token header
    src.push_str(
        "            request = request.header(\"x-ignition-token\", self.config.token.clone());\n",
    );

    // Add request body if needed
    if let Some(request) = &method.request {
        match request {
            ApiRequest::SchemaDefinition { name } => {
                src.push_str(&format!(
                    "            let response = request.json(&{}).send().await?;\n",
                    name.to_lowercase()
                ));
            }
            ApiRequest::OptionalSchemaDefinition { name } => {
                src.push_str(&format!(
                    "            if let Some({}) = {} {{\n",
                    name.to_lowercase(),
                    name.to_lowercase()
                ));
                src.push_str(&format!(
                    "                request = request.json(&{});\n",
                    name.to_lowercase()
                ));
                src.push_str("            }\n");
                src.push_str("            let response = request.send().await?;\n");
            }
            ApiRequest::TaggedSchemaDefinition { name, .. } => {
                src.push_str(&format!(
                    "            let response = request.json(&{}).send().await?;\n",
                    name.to_lowercase()
                ));
            }
        }
    } else {
        src.push_str("            let response = request.send().await?;\n");
    }

    // Handle response
    if let Some(_) = &method.response {
        let response_inner_type = generate_response_inner_type(service, &method.response);
        src.push_str("            let bytes = response.bytes().await?;\n");
        src.push_str(&format!(
            "            let result: Result<{}, _> = serde_json::from_slice(&bytes);\n",
            response_inner_type
        ));
        src.push_str("            match result {\n");
        src.push_str("                Ok(val) => Ok(val),\n");
        src.push_str("                Err(e) => {\n");
        src.push_str("                    let response_text = String::from_utf8_lossy(&bytes);\n");
        src.push_str("                    Err(anyhow::anyhow!(\"{}\", response_text))\n");
        src.push_str("                }\n");
        src.push_str("            }\n");
    } else {
        src.push_str("            if !response.status().is_success() {\n");
        src.push_str("                return Err(anyhow::anyhow!(\n");
        src.push_str(&format!(
            "                    \"failed to {}: {{}}\",\n",
            method_name
        ));
        src.push_str("                    response.text().await?\n");
        src.push_str("                ));\n");
        src.push_str("            }\n");
        src.push_str("            Ok(())\n");
    }

    src.push_str("        }\n\n");
}

fn generate_websocket_method(src: &mut String, service: &ApiService, method: &ApiMethod) {
    let method_name = &method.name;

    if method.verb != ApiVerb::WebSocket {
        unreachable!();
    }

    let mut params = Vec::new();
    if service.namespaced || method.namespaced {
        params.push("namespace: Namespace".to_string());
    }
    if let Some(request) = &method.request {
        match request {
            ApiRequest::SchemaDefinition { name } => {
                params.push(format!("opts: crate::{}::{}", service.crate_path, name));
            }
            _ => unreachable!(),
        }
    }

    src.push_str(&format!(
        "        pub async fn {}(&self, {}) -> Result<IgnitionWsStream<{}>> {{\n",
        method_name,
        params.join(", "),
        generate_response_inner_type(service, &method.response)
    ));

    // Build URL with query parameters
    src.push_str(&format!(
        "            let mut url = format!(\"{{}}/{}\", self.config.base_url);\n",
        method
            .path
            .iter()
            .map(|s| match s {
                ApiPathSegment::Static { value } => value.to_string(),
                ApiPathSegment::ResourceName => "{}".to_string(),
            })
            .collect::<Vec<_>>()
            .join("/")
    ));

    // Add query parameters if opts is present
    if let Some(_) = &method.request {
        src.push_str("            let query_params = serde_urlencoded::to_string(&opts)?;\n");
        src.push_str("            if !query_params.is_empty() {\n");
        src.push_str("                url.push('?');\n");
        src.push_str("                url.push_str(&query_params);\n");
        src.push_str("            }\n");
    }

    // Convert HTTP URL to WebSocket URL
    src.push_str("            let ws_url = url.replace(\"http://\", \"ws://\").replace(\"https://\", \"wss://\");\n");

    // Build request with headers
    src.push_str("            let mut request = ws_url.into_client_request()?;\n");
    src.push_str("            let headers = request.headers_mut();\n");
    src.push_str("            headers.insert(\"x-ignition-token\", HeaderValue::from_str(&self.config.token)?);\n");

    // Add namespace header if namespaced
    let namespaced = service.namespaced || method.namespaced;
    if namespaced {
        src.push_str("            if let Some(namespace) = namespace.as_value() {\n");
        src.push_str("                headers.insert(\"x-ignition-namespace\", HeaderValue::from_str(&namespace)?);\n");
        src.push_str("            }\n");
    }

    src.push_str(
        "            let (ws_stream, _) = tokio_tungstenite::connect_async(request).await?;\n",
    );

    src.push_str("            Ok(IgnitionWsStream::new(ws_stream))\n");
    src.push_str("        }\n\n");
}
