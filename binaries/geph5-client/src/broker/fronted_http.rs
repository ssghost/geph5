use std::time::Instant;

use async_trait::async_trait;
use nanorpc::{JrpcRequest, JrpcResponse, RpcTransport};
use reqwest::Client;

pub struct FrontedHttpTransport {
    pub url: String,
    pub host: Option<String>,
    pub client: Client,
}

#[async_trait]
impl RpcTransport for FrontedHttpTransport {
    type Error = anyhow::Error;
    async fn call_raw(&self, req: JrpcRequest) -> Result<JrpcResponse, Self::Error> {
        tracing::trace!(method = req.method, "calling broker");
        let start = Instant::now();
        let mut request_builder = self
            .client
            .post(&self.url)
            .header("content-type", "application/json");

        if let Some(host) = &self.host {
            request_builder = request_builder.header("Host", host);
        }

        let request_body = serde_json::to_vec(&req)?;
        let response = request_builder.body(request_body).send().await?;

        let resp_bytes = response.bytes().await?;
        tracing::trace!(
            method = req.method,
            resp_len = resp_bytes.len(),
            elapsed = debug(start.elapsed()),
            "response received"
        );
        Ok(serde_json::from_slice(&resp_bytes)?)
    }
}
