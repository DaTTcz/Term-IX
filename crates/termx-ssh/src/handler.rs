use async_trait::async_trait;
use russh::client;
use russh_keys::key::PublicKey;

/// MVP handler: prijme jakykoliv klic serveru bez overeni (Trust On First Use
/// jeste ani neni implementovano, natoz plnohodnotny known_hosts).
///
/// TODO (pred pouzitim na produkcnich/verejnych serverech):
/// - ulozit otisk klice pri prvnim pripojeni (napr. do AppPaths::config_dir())
/// - pri dalsich pripojenich jej porovnat a upozornit uzivatele pri zmene
pub struct TofuHandler;

#[async_trait]
impl client::Handler for TofuHandler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, _server_public_key: &PublicKey) -> Result<bool, Self::Error> {
        Ok(true)
    }
}
