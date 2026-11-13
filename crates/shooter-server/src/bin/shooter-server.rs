//! Hosts deathmatches over WebTransport (game-server). Configured by environment:
//!
//! | Variable | Default | |
//! | --- | --- | --- |
//! | `SHOOTER_PORT` | 4433 | UDP port |
//! | `SHOOTER_CERT_PEM`, `SHOOTER_KEY_PEM` | `cert.pem`, `key.pem` | TLS identity |
//! | `SHOOTER_MATCH_IDS` | `dm` | comma-separated matches, one deathmatch each: `<id>` (on the greybox) or `<id>=<map>` |
//! | `SHOOTER_ROUTE_PREFIX` | `/shooter` | matches are at `<prefix>/matches/<id>` |
//! | `SHOOTER_ALLOWED_ORIGINS` | none | comma-separated browser origins (unset: any client) |
//!
//! `scripts/dev.sh` makes a development certificate and runs this with a native client.

use game_server::{
    BrowserOriginAllowlist, BrowserRoutePrefix, DEFAULT_RECONNECT_GRACE_TICKS,
    MatchHostWebTransportConfig, serve_match_host_with_shutdown,
};
use shooter_server::{MatchSpec, build_host};
use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;

fn setting(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let port: u16 = setting("SHOOTER_PORT", "4433").parse()?;
    let matches = list(&setting("SHOOTER_MATCH_IDS", "dm"))
        .iter()
        .map(|spec| spec.parse())
        .collect::<Result<Vec<MatchSpec>, _>>()?;
    let prefix = setting("SHOOTER_ROUTE_PREFIX", "/shooter");
    let origins = list(&setting("SHOOTER_ALLOWED_ORIGINS", ""));
    let host = build_host(&matches, DEFAULT_RECONNECT_GRACE_TICKS)?;
    let config = MatchHostWebTransportConfig {
        port,
        certificate_pem: PathBuf::from(setting("SHOOTER_CERT_PEM", "cert.pem")),
        private_key_pem: PathBuf::from(setting("SHOOTER_KEY_PEM", "key.pem")),
        route_prefix: BrowserRoutePrefix::new(prefix.clone())?,
        allowed_origins: if origins.is_empty() {
            None
        } else {
            Some(BrowserOriginAllowlist::new(origins)?)
        },
        drain_grace: Duration::from_millis(500),
    };
    for spec in &matches {
        println!(
            "deathmatch on {} ({} players recommended) at https://localhost:{port}{prefix}/matches/{}",
            spec.map,
            spec.map.capacity(),
            spec.id
        );
    }
    let (shutdown, requests) = mpsc::channel(1);
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = shutdown.send(()).await;
        }
    });
    serve_match_host_with_shutdown(host, config, requests).await?;
    Ok(())
}
