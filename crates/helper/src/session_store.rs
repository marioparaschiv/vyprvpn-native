use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tunnel::{ConnectionId, KeyError, PresharedKey};

/// One server-side WireGuard session. Its preshared key authorizes deleting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub spoke_id: String,
    pub connection_id: ConnectionId,
    pub preshared_key: PresharedKey,
}

/// What outlives the helper: the location last connected or attempted, the session while one
/// may exist on the server, and sessions whose deletion failed and is still owed. Never the
/// private key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Saved {
    pub last_spoke_id: Option<String>,
    pub session: Option<Session>,
    pub pending_deletions: Vec<Session>,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionStoreError {
    #[error("accessing {path} failed: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{path} is malformed: {source}")]
    Malformed {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("{path} holds an invalid preshared key: {source}")]
    PresharedKey { path: PathBuf, source: KeyError },
}

#[derive(Serialize, Deserialize)]
struct SavedFile {
    last_spoke_id: Option<String>,
    session: Option<SessionFile>,
    #[serde(default)]
    pending_deletions: Vec<SessionFile>,
}

#[derive(Serialize, Deserialize)]
struct SessionFile {
    spoke_id: String,
    connection_id: String,
    preshared_key: String,
}

/// `connection.json` in the helper's data directory, which only the helper's account can read.
#[derive(Debug, Clone)]
pub struct SessionStore {
    path: PathBuf,
}

impl SessionStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join("connection.json"),
        }
    }

    pub fn load(&self) -> Result<Saved, SessionStoreError> {
        let json = match fs::read(&self.path) {
            Ok(json) => json,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Saved::default()),
            Err(source) => return Err(self.io_error(source)),
        };
        let file: SavedFile =
            serde_json::from_slice(&json).map_err(|source| SessionStoreError::Malformed {
                path: self.path.clone(),
                source,
            })?;

        let session = |session: SessionFile| {
            let preshared_key = session.preshared_key.parse().map_err(|source| {
                SessionStoreError::PresharedKey {
                    path: self.path.clone(),
                    source,
                }
            })?;

            Ok(Session {
                spoke_id: session.spoke_id,
                connection_id: ConnectionId(session.connection_id),
                preshared_key,
            })
        };

        Ok(Saved {
            last_spoke_id: file.last_spoke_id,
            session: file.session.map(session).transpose()?,
            pending_deletions: file
                .pending_deletions
                .into_iter()
                .map(session)
                .collect::<Result<_, _>>()?,
        })
    }

    /// Replaces the file in one step, so a crash never leaves half of it behind.
    pub fn save(&self, saved: &Saved) -> Result<(), SessionStoreError> {
        let session = |session: &Session| SessionFile {
            spoke_id: session.spoke_id.clone(),
            connection_id: session.connection_id.0.clone(),
            preshared_key: session.preshared_key.to_base64().to_string(),
        };
        let file = SavedFile {
            last_spoke_id: saved.last_spoke_id.clone(),
            session: saved.session.as_ref().map(session),
            pending_deletions: saved.pending_deletions.iter().map(session).collect(),
        };
        let json = serde_json::to_vec(&file).map_err(|source| SessionStoreError::Malformed {
            path: self.path.clone(),
            source,
        })?;
        let partial = self.path.with_extension("json.partial");

        fs::write(&partial, json).map_err(|source| self.io_error(source))?;
        fs::rename(&partial, &self.path).map_err(|source| self.io_error(source))
    }

    fn io_error(&self, source: io::Error) -> SessionStoreError {
        SessionStoreError::Io {
            path: self.path.clone(),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRESHARED: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

    fn session(connection_id: &str) -> Session {
        Session {
            spoke_id: "de1".into(),
            connection_id: ConnectionId(connection_id.into()),
            preshared_key: PRESHARED.parse().unwrap(),
        }
    }

    fn connected_to_de1() -> Saved {
        Saved {
            last_spoke_id: Some("de1".into()),
            session: Some(session("48213977")),
            pending_deletions: vec![session("48213900")],
        }
    }

    #[test]
    fn loads_nothing_before_the_first_save() {
        let data_dir = tempfile::tempdir().unwrap();

        assert_eq!(
            SessionStore::new(data_dir.path()).load().unwrap(),
            Saved::default()
        );
    }

    #[test]
    fn round_trips_the_session_and_last_location() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(data_dir.path());

        store.save(&connected_to_de1()).unwrap();

        assert_eq!(store.load().unwrap(), connected_to_de1());
        assert_eq!(
            fs::read_to_string(data_dir.path().join("connection.json")).unwrap(),
            format!(
                r#"{{"last_spoke_id":"de1","session":{{"spoke_id":"de1","connection_id":"48213977","preshared_key":"{PRESHARED}"}},"pending_deletions":[{{"spoke_id":"de1","connection_id":"48213900","preshared_key":"{PRESHARED}"}}]}}"#
            )
        );
    }

    #[test]
    fn overwrites_the_previous_save() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(data_dir.path());
        store.save(&connected_to_de1()).unwrap();

        let disconnected = Saved {
            last_spoke_id: Some("de1".into()),
            ..Saved::default()
        };
        store.save(&disconnected).unwrap();

        assert_eq!(store.load().unwrap(), disconnected);
        assert_eq!(fs::read_dir(data_dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn rejects_a_malformed_file() {
        let data_dir = tempfile::tempdir().unwrap();
        fs::write(data_dir.path().join("connection.json"), "{").unwrap();

        assert!(matches!(
            SessionStore::new(data_dir.path()).load(),
            Err(SessionStoreError::Malformed { .. })
        ));
    }
}
