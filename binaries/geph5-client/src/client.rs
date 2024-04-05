use anyctx::AnyCtx;
use clone_macro::clone;
use futures_util::{
    future::{FusedFuture, Shared},
    task::noop_waker,
    FutureExt, TryFutureExt,
};
use geph5_broker_protocol::{Credential, ExitList};
use smol::future::FutureExt as _;
use std::{net::SocketAddr, path::PathBuf, sync::Arc, task::Context, time::Duration};

use serde::{Deserialize, Serialize};
use smolscale::immortal::{Immortal, RespawnStrategy};

use crate::{
    auth::auth_loop,
    broker::{broker_client, BrokerSource},
    client_inner::client_once,
    database::db_read_or_wait,
    route::ExitConstraint,
    socks5::socks5_loop,
};

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub socks5_listen: SocketAddr,
    pub exit_constraint: ExitConstraint,
    pub cache: Option<PathBuf>,
    pub broker: Option<BrokerSource>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub credentials: Credential,
}

pub struct Client {
    task: Shared<smol::Task<Result<(), Arc<anyhow::Error>>>>,
}

impl Client {
    /// Starts the client logic in the loop, returnign the handle.
    pub fn start(cfg: Config) -> Self {
        let ctx = AnyCtx::new(cfg);
        let task = smolscale::spawn(client_main(ctx).map_err(Arc::new));
        Client {
            task: task.shared(),
        }
    }

    /// Wait until there's an error.
    pub async fn wait_until_dead(self) -> anyhow::Result<()> {
        self.task.await.map_err(|e| anyhow::anyhow!(e))
    }

    /// Check for an error.
    pub fn check_dead(&self) -> anyhow::Result<()> {
        match self
            .task
            .clone()
            .poll(&mut Context::from_waker(&noop_waker()))
        {
            std::task::Poll::Ready(val) => val.map_err(|e| anyhow::anyhow!(e))?,
            std::task::Poll::Pending => {}
        }

        Ok(())
    }
}

pub type CtxField<T> = fn(&AnyCtx<Config>) -> T;

async fn client_main(ctx: AnyCtx<Config>) -> anyhow::Result<()> {
    #[derive(Serialize)]
    struct DryRunOutput {
        auth_token: String,
        exits: ExitList,
    }

    if ctx.init().dry_run {
        auth_loop(&ctx)
            .race(async {
                let broker_client = broker_client(&ctx)?;
                let exits = broker_client
                    .get_exits()
                    .await?
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                let auth_token = db_read_or_wait(&ctx, "auth_token").await?;
                let exits = exits.inner;
                println!(
                    "{}",
                    serde_json::to_string(&DryRunOutput {
                        auth_token: hex::encode(auth_token),
                        exits,
                    })?
                );
                anyhow::Ok(())
            })
            .await
    } else {
        let _client_loop = Immortal::respawn(
            RespawnStrategy::JitterDelay(Duration::from_secs(1), Duration::from_secs(5)),
            clone!([ctx], move || client_once(ctx.clone()).inspect_err(
                |e| tracing::warn!("client died and restarted: {:?}", e)
            )),
        );
        socks5_loop(&ctx).race(auth_loop(&ctx)).await
    }
}
