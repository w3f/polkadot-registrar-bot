#![recursion_limit = "512"]

#[macro_use]
extern crate log;
#[macro_use]
extern crate async_trait;
#[macro_use]
extern crate serde;
#[macro_use]
extern crate failure;

use adapters::MatrixClient;
use connector::Connector;
use db::Database;
use identity::IdentityManager;
use primitives::{AccountType, Result};
use tokio::time::{self, Duration};

// TODO: Make private
pub mod adapters;
mod comms;
mod connector;
mod db;
mod identity;
mod primitives;
mod verifier;

pub struct Config {
    pub db_path: String,
    pub watcher_url: String,
    pub matrix_homeserver: String,
    pub matrix_username: String,
    pub matrix_password: String,
}

pub async fn run(config: Config) -> Result<()> {
    info!("Setting up database and manager");
    let db = Database::new(&config.db_path)?;
    let mut manager = IdentityManager::new(db)?;

    info!("Setting up communication channels");
    let c_connector = manager.register_comms(AccountType::ReservedConnector);
    let c_emitter = manager.register_comms(AccountType::ReservedEmitter);
    let c_matrix = manager.register_comms(AccountType::Matrix);
    // TODO: move to a test suite
    let c_test = manager.register_comms(AccountType::Email);

    let mut interval = time::interval(Duration::from_secs(5));
    let connector;
    info!("Trying to connect to Watcher");

    let mut counter = 0;
    loop {
        interval.tick().await;

        if let Ok(con) = Connector::new(&config.watcher_url, c_connector.clone()).await {
            info!("Connecting to Watcher succeeded");
            connector = con;
            break;
        } else {
            warn!("Connecting to Watcher failed, trying again...");
        }

        if counter == 2 {
            panic!("Failed connecting to Watcher")
        }

        counter += 1;
    }

    info!("Setting up Matrix client");
    let matrix = MatrixClient::new(
        &config.matrix_homeserver,
        &config.matrix_username,
        &config.matrix_password,
        c_matrix,
        //c_matrix_emitter,
        c_emitter,
    )
    .await?;

    // TODO: move to a test suite
    identity::TestClient::new(c_test).gen_data();

    info!("Starting all tasks...");
    tokio::spawn(async move {
        manager.start().await.unwrap();
    });
    tokio::spawn(async move {
        connector.start().await;
    });
    tokio::spawn(async move {
        matrix.start().await;
    });

    info!("All tasks executed");

    // TODO: Adjust this
    let mut interval = time::interval(Duration::from_secs(60));
    loop {
        interval.tick().await;
    }
}
