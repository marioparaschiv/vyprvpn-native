use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use ipc::{Account, Credentials, Location, Status};
use tokio::sync::{OnceCell, watch};

use crate::credential_store::{CredentialStore, CredentialStoreError};
use crate::vpn::{Realm, Tunnel, Vpn};

#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error(transparent)]
    Api(#[from] vypr_api::Error),
    #[error("storing the login failed: {0}")]
    Store(#[from] CredentialStoreError),
}

impl LoginError {
    /// What the login window shows; the details belong in the helper's log.
    pub fn user_message(&self) -> String {
        match self {
            Self::Api(vypr_api::Error::Network(_)) => {
                "Couldn't reach VyprVPN. Check your internet connection.".to_owned()
            }
            Self::Api(vypr_api::Error::Http { .. } | vypr_api::Error::Decode { .. }) => {
                "VyprVPN returned an unexpected response.".to_owned()
            }
            Self::Api(error) => error.to_string(),
            Self::Store(_) => "Saving the login failed.".to_owned(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("not logged in")]
    LoggedOut,
    #[error("reading the stored login failed: {0}")]
    Store(#[from] CredentialStoreError),
}

/// What every tray app connection shares.
pub struct State<T: Tunnel> {
    api: vypr_api::Client,
    vpn: Arc<Vpn<vypr_api::Client, T>>,
    locations: OnceCell<Vec<Location>>,
    credentials: CredentialStore,
    /// Known from the login; after a restart the first connect fetches it.
    realm: Mutex<Realm>,
    account: OnceLock<watch::Sender<Account>>,
}

impl<T: Tunnel> State<T> {
    pub fn new(api_base_url: &str, data_dir: &Path, tunnel: T) -> Result<Self, vypr_api::Error> {
        let api = vypr_api::Client::new(api_base_url)?;

        Ok(Self {
            vpn: Vpn::new(api.clone(), tunnel, data_dir),
            api,
            locations: OnceCell::new(),
            credentials: CredentialStore::new(data_dir),
            realm: Mutex::default(),
            account: OnceLock::new(),
        })
    }

    pub fn status(&self) -> watch::Receiver<Status> {
        self.vpn.subscribe()
    }

    /// Reads the stored login on first use, so startup never waits on the disk or DPAPI.
    pub fn account(&self) -> &watch::Sender<Account> {
        self.account.get_or_init(|| {
            let account = match self.credentials.load() {
                Ok(Some(credentials)) => Account::LoggedIn {
                    username: credentials.username,
                },
                Ok(None) => Account::LoggedOut,
                Err(error) => {
                    log::error!(
                        "reading the stored login failed; treating it as logged out: {error}"
                    );
                    Account::LoggedOut
                }
            };

            watch::Sender::new(account)
        })
    }

    /// Fetched once; a failed fetch is retried on the next call.
    pub async fn locations(&self) -> Result<Vec<Location>, vypr_api::Error> {
        self.locations
            .get_or_try_init(|| self.api.locations())
            .await
            .cloned()
    }

    pub async fn login(&self, credentials: Credentials) -> Result<Account, LoginError> {
        let realm = self.api.settings(&credentials).await?.realm;
        self.credentials.save(&credentials)?;
        *self.realm() = Arc::new(OnceCell::from(realm));

        log::info!("logged in as {}", credentials.username);
        Ok(self.publish(Account::LoggedIn {
            username: credentials.username,
        }))
    }

    /// Answers with `connecting`; the outcome follows as status events.
    pub fn connect(&self, spoke_id: String) -> Result<Status, ConnectError> {
        let credentials = self.credentials.load()?.ok_or(ConnectError::LoggedOut)?;

        Ok(self
            .vpn
            .connect(spoke_id, credentials, self.realm().clone()))
    }

    pub fn disconnect(&self) -> Status {
        self.vpn.disconnect()
    }

    pub fn logout(&self) -> Result<Account, CredentialStoreError> {
        self.vpn.disconnect();
        self.credentials.delete()?;
        *self.realm() = Realm::default();

        log::info!("logged out");
        Ok(self.publish(Account::LoggedOut))
    }

    fn realm(&self) -> MutexGuard<'_, Realm> {
        self.realm.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn publish(&self, account: Account) -> Account {
        self.account().send_replace(account.clone());

        account
    }
}
