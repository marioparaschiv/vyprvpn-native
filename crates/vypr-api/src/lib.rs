//! Client for the VyprVPN HTTP API.

#[cfg(not(any(windows, target_os = "macos")))]
compile_error!("vypr-api supports only Windows and macOS");

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use std::fmt;
use std::time::Duration;

pub use locations::Location;
use md5::{Digest, Md5};
pub use reqwest::StatusCode;
use reqwest::header::{
    CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, InvalidHeaderValue, USER_AGENT,
};
use reqwest::{Method, RequestBuilder, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tunnel::{ConnectResponse, ConnectionId, PresharedKey, PublicKey};

#[cfg(target_os = "macos")]
use macos::{OsVersionError, PLATFORM, os_version};
#[cfg(windows)]
use windows::{OsVersionError, PLATFORM, os_version};

pub const DEFAULT_BASE_URL: &str = "https://api.vyprvpn.com";
const PRODUCT_VERSION: &str = "6.0.4";
const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid username or password")]
    InvalidCredentials,
    #[error("VyprVPN answered {path} with HTTP {status}: {body}")]
    Http {
        path: String,
        status: StatusCode,
        body: String,
    },
    #[error("VyprVPN sent an unexpected response to {path}: {source}")]
    Decode {
        path: String,
        source: serde_json::Error,
    },
    #[error("could not reach VyprVPN: {0}")]
    Network(#[from] reqwest::Error),
    #[error("invalid API base URL {url:?}: {source}")]
    BaseUrl {
        url: String,
        source: url::ParseError,
    },
    #[error("a username, password or realm contains characters that cannot be sent in a header")]
    HeaderValue(#[from] InvalidHeaderValue),
    #[error("could not read the OS version: {0}")]
    OsVersion(#[from] OsVersionError),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Settings {
    pub realm: String,
}

/// A VyprVPN login. `Debug` never prints the password.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: Url,
}

impl Client {
    /// `base_url` is [`DEFAULT_BASE_URL`] except in tests.
    pub fn new(base_url: &str) -> Result<Self, Error> {
        let base_url = Url::parse(base_url).map_err(|source| Error::BaseUrl {
            url: base_url.to_owned(),
            source,
        })?;

        let http = reqwest::Client::builder()
            .default_headers(standard_headers()?)
            .timeout(TIMEOUT)
            .build()?;

        Ok(Self { http, base_url })
    }

    pub async fn locations(&self) -> Result<Vec<Location>, Error> {
        #[derive(Deserialize)]
        struct Body {
            locations: Vec<Location>,
        }

        let path = "/vyprvpn/locations-unauthenticated";
        let request = self.get(path).header("locale", "en_US");
        let body: Body = decode(path, &send(path, request).await?)?;

        Ok(body.locations)
    }

    /// Doubles as the login check: wrong credentials give [`Error::InvalidCredentials`].
    pub async fn settings(&self, credentials: &Credentials) -> Result<Settings, Error> {
        let path = "/settings";
        let request = self.get(path).headers(auth_headers(credentials)?);

        decode(path, &send_authenticated(path, request).await?)
    }

    /// Opens a session on a location. `realm` is the one [`Client::settings`] returns.
    pub async fn connect(
        &self,
        credentials: &Credentials,
        realm: &str,
        spoke_id: &str,
        client_key: &PublicKey,
    ) -> Result<ConnectResponse, Error> {
        let path = connections_path(spoke_id);
        let body = json!({ "Server": spoke_id, "ClientKey": client_key.to_string() });
        let request = self
            .request(Method::POST, &path)
            .headers(auth_headers(credentials)?)
            .header("x-gf-realm", HeaderValue::try_from(realm)?)
            .body(body.to_string());

        decode(&path, &send_authenticated(&path, request).await?)
    }

    /// Closes a session. The preshared key authorizes this in place of the login.
    pub async fn disconnect(
        &self,
        spoke_id: &str,
        connection_id: &ConnectionId,
        preshared_key: &PresharedKey,
    ) -> Result<(), Error> {
        let path = format!("{}/{}", connections_path(spoke_id), connection_id.0);
        let mut psk = HeaderValue::try_from(preshared_key.to_base64().as_str())?;
        psk.set_sensitive(true);
        let request = self
            .request(Method::DELETE, &path)
            .header("wireguard-psk", psk);

        send(&path, request).await?;

        Ok(())
    }

    /// The 64-pixel PNG flag for a location's three-letter `country_code`.
    pub async fn flag(&self, country_code: &str) -> Result<Vec<u8>, Error> {
        let path = format!(
            "/images/vpn_flags/64/{}.png",
            country_code.to_ascii_lowercase()
        );

        send(&path, self.get(&path)).await
    }

    fn get(&self, path: &str) -> RequestBuilder {
        self.request(Method::GET, path)
    }

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        let mut url = self.base_url.clone();
        url.set_path(path);

        self.http.request(method, url)
    }
}

fn connections_path(spoke_id: &str) -> String {
    format!("/vyprvpn/spokes/{spoke_id}/wg/connections")
}

/// VyprVPN refuses wrong credentials with 401 or 403.
async fn send_authenticated(path: &str, request: RequestBuilder) -> Result<Vec<u8>, Error> {
    send(path, request).await.map_err(|error| match error {
        Error::Http {
            status: StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN,
            ..
        } => Error::InvalidCredentials,
        error => error,
    })
}

async fn send(path: &str, request: RequestBuilder) -> Result<Vec<u8>, Error> {
    let response = request.send().await?;
    let status = response.status();
    let body = response.bytes().await?;

    if !status.is_success() {
        return Err(Error::Http {
            path: path.to_owned(),
            status,
            body: String::from_utf8_lossy(&body).trim().to_owned(),
        });
    }

    Ok(body.to_vec())
}

fn decode<T: DeserializeOwned>(path: &str, body: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(body).map_err(|source| Error::Decode {
        path: path.to_owned(),
        source,
    })
}

fn standard_headers() -> Result<HeaderMap, OsVersionError> {
    let os_version = os_version()?;
    let header = |value: String| {
        HeaderValue::try_from(value).expect("the platform and version names are ASCII")
    };

    Ok(HeaderMap::from_iter([
        (CONTENT_TYPE, HeaderValue::from_static("application/json")),
        (
            USER_AGENT,
            header(format!(
                "{PLATFORM}/{os_version} VyprVPN/{PRODUCT_VERSION}.0"
            )),
        ),
        (
            HeaderName::from_static("x-gf-platform"),
            HeaderValue::from_static(PLATFORM),
        ),
        (
            HeaderName::from_static("x-gf-product"),
            HeaderValue::from_static("VyprVPN"),
        ),
        (
            HeaderName::from_static("x-gf-platform-version"),
            header(os_version),
        ),
        (
            HeaderName::from_static("x-gf-product-version"),
            HeaderValue::from_static(PRODUCT_VERSION),
        ),
    ]))
}

fn auth_headers(credentials: &Credentials) -> Result<HeaderMap, InvalidHeaderValue> {
    let mut password = HeaderValue::try_from(credentials.password.as_str())?;
    password.set_sensitive(true);
    let agent = format!(
        "VyprVPN {PLATFORM} {PRODUCT_VERSION}.0 ({})",
        agent_digest(credentials)
    );

    Ok(HeaderMap::from_iter([
        (
            HeaderName::from_static("username"),
            HeaderValue::try_from(credentials.username.as_str())?,
        ),
        (HeaderName::from_static("password"), password),
        (
            HeaderName::from_static("x-gf-agent"),
            HeaderValue::try_from(agent)?,
        ),
    ]))
}

/// The last 8 uppercase hex digits of `MD5("VyprVPN" + username + password)`, i.e. its last 4 bytes.
fn agent_digest(credentials: &Credentials) -> String {
    let digest = Md5::new()
        .chain_update("VyprVPN")
        .chain_update(&credentials.username)
        .chain_update(&credentials.password)
        .finalize();

    digest[12..]
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockBuilder, MockServer, ResponseTemplate};

    use super::*;

    const LOCATIONS: &str = include_str!("../tests/fixtures/locations.json");
    const SETTINGS: &str = include_str!("../tests/fixtures/settings.json");
    const CONNECT: &str = include_str!("../tests/fixtures/connect.json");
    const CLIENT_KEY: &str = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=";
    const PRESHARED: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
    const CONNECTIONS: &str = "/vyprvpn/spokes/uk1/wg/connections";

    fn alice() -> Credentials {
        Credentials {
            username: "alice".into(),
            password: "correct horse".into(),
        }
    }

    fn alice_agent() -> String {
        format!("VyprVPN {PLATFORM} 6.0.4.0 (4FA59B8E)")
    }

    fn client_for(server: &MockServer) -> Client {
        Client::new(&server.uri()).unwrap()
    }

    fn with_standard_headers(mock: MockBuilder) -> MockBuilder {
        let os_version = os_version().unwrap();

        mock.and(header("content-type", "application/json"))
            .and(header(
                "user-agent",
                format!("{PLATFORM}/{os_version} VyprVPN/6.0.4.0"),
            ))
            .and(header("x-gf-platform", PLATFORM))
            .and(header("x-gf-product", "VyprVPN"))
            .and(header("x-gf-platform-version", os_version))
            .and(header("x-gf-product-version", "6.0.4"))
    }

    async fn connect_to_uk1(client: &Client) -> Result<ConnectResponse, Error> {
        client
            .connect(&alice(), "vyprvpn", "uk1", &CLIENT_KEY.parse().unwrap())
            .await
    }

    async fn disconnect_from_uk1(client: &Client) -> Result<(), Error> {
        client
            .disconnect(
                "uk1",
                &ConnectionId("48213977".into()),
                &PRESHARED.parse().unwrap(),
            )
            .await
    }

    #[test]
    fn agent_digest_matches_the_official_client() {
        assert_eq!(agent_digest(&alice()), "4FA59B8E");
        assert_eq!(
            agent_digest(&Credentials {
                username: "user@example.com".into(),
                password: "hunter2".into(),
            }),
            "329E2894"
        );
    }

    #[test]
    fn debug_hides_the_password() {
        let printed = format!("{:?}", alice());

        assert!(printed.contains("alice"), "{printed}");
        assert!(!printed.contains("horse"), "{printed}");
    }

    #[test]
    fn rejects_a_base_url_that_is_not_a_url() {
        assert!(matches!(
            Client::new("api.vyprvpn.com"),
            Err(Error::BaseUrl { .. })
        ));
    }

    #[tokio::test]
    async fn fetches_locations_with_the_standard_headers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/vyprvpn/locations-unauthenticated"))
            .and(header("locale", "en_US"))
            .and(header("x-gf-platform", PLATFORM))
            .and(header("x-gf-product", "VyprVPN"))
            .and(header("x-gf-product-version", "6.0.4"))
            .respond_with(ResponseTemplate::new(200).set_body_string(LOCATIONS))
            .expect(1)
            .mount(&server)
            .await;

        let locations = client_for(&server).locations().await.unwrap();

        assert_eq!(locations.len(), 73);
        assert_eq!(
            locations[0],
            Location {
                name: "United Kingdom".into(),
                country: "United Kingdom".into(),
                country_code: "GBR".into(),
                region: "Europe".into(),
                spoke_id: "uk1".into(),
                hostname: "uk1.vpn.goldenfrog.com".into(),
            }
        );
    }

    #[tokio::test]
    async fn sends_credentials_and_agent_to_settings() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/settings"))
            .and(header("username", "alice"))
            .and(header("password", "correct horse"))
            .and(header("x-gf-agent", alice_agent()))
            .respond_with(ResponseTemplate::new(200).set_body_string(SETTINGS))
            .expect(1)
            .mount(&server)
            .await;

        let settings = client_for(&server).settings(&alice()).await.unwrap();

        assert_eq!(settings.realm, "vyprvpn");
    }

    #[tokio::test]
    async fn maps_rejected_logins_to_invalid_credentials() {
        for status in [401, 403] {
            let server = MockServer::start().await;
            Mock::given(path("/settings"))
                .respond_with(ResponseTemplate::new(status).set_body_string("Forbidden"))
                .mount(&server)
                .await;

            let error = client_for(&server).settings(&alice()).await.unwrap_err();

            assert!(
                matches!(error, Error::InvalidCredentials),
                "{status}: {error}"
            );
            assert_eq!(error.to_string(), "invalid username or password");
        }
    }

    #[tokio::test]
    async fn reports_other_failures_with_the_status_and_body() {
        let server = MockServer::start().await;
        Mock::given(path("/vyprvpn/locations-unauthenticated"))
            .respond_with(ResponseTemplate::new(503).set_body_string("down for maintenance\n"))
            .mount(&server)
            .await;

        let error = client_for(&server).locations().await.unwrap_err();

        assert_eq!(
            error.to_string(),
            "VyprVPN answered /vyprvpn/locations-unauthenticated with HTTP 503 Service Unavailable: down for maintenance"
        );
    }

    #[tokio::test]
    async fn reports_a_malformed_body() {
        let server = MockServer::start().await;
        Mock::given(path("/settings"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
            .mount(&server)
            .await;

        let error = client_for(&server).settings(&alice()).await.unwrap_err();

        assert!(matches!(error, Error::Decode { .. }), "{error}");
    }

    #[tokio::test]
    async fn downloads_flags_by_lowercase_country_code() {
        let server = MockServer::start().await;
        Mock::given(path("/images/vpn_flags/64/ger.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"\x89PNG".to_vec()))
            .mount(&server)
            .await;

        let flag = client_for(&server).flag("GER").await.unwrap();

        assert_eq!(flag, b"\x89PNG");
    }

    #[tokio::test]
    async fn connect_posts_the_client_key_with_login_and_realm() {
        let server = MockServer::start().await;
        with_standard_headers(Mock::given(method("POST")).and(path(CONNECTIONS)))
            .and(header("username", "alice"))
            .and(header("password", "correct horse"))
            .and(header("x-gf-agent", alice_agent()))
            .and(header("x-gf-realm", "vyprvpn"))
            .and(body_json(
                json!({ "Server": "uk1", "ClientKey": CLIENT_KEY }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(CONNECT))
            .expect(1)
            .mount(&server)
            .await;

        let response = connect_to_uk1(&client_for(&server)).await.unwrap();

        assert_eq!(response.connection_id, ConnectionId("48213977".into()));
        assert_eq!(response.interface.address, "10.21.0.7/32");
        assert_eq!(response.interface.dns, ["10.21.0.1", "10.21.0.2"]);
        assert_eq!(
            response.peer.public_key,
            "hSDwCYkwp1R0i33ctD73Wg2/Og0mOBr066SpjqqbTmo="
        );
        assert_eq!(response.peer.preshared_key, PRESHARED);
        assert_eq!(response.peer.endpoint, "203.0.113.10:51820");
    }

    #[tokio::test]
    async fn connect_maps_rejected_logins_to_invalid_credentials() {
        for status in [401, 403] {
            let server = MockServer::start().await;
            Mock::given(path(CONNECTIONS))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;

            let error = connect_to_uk1(&client_for(&server)).await.unwrap_err();

            assert!(
                matches!(error, Error::InvalidCredentials),
                "{status}: {error}"
            );
        }
    }

    #[tokio::test]
    async fn connect_reports_server_errors() {
        let server = MockServer::start().await;
        Mock::given(path(CONNECTIONS))
            .respond_with(ResponseTemplate::new(500).set_body_string("spoke unavailable"))
            .mount(&server)
            .await;

        let error = connect_to_uk1(&client_for(&server)).await.unwrap_err();

        assert!(
            matches!(
                &error,
                Error::Http { path, status: StatusCode::INTERNAL_SERVER_ERROR, body }
                    if path == CONNECTIONS && body == "spoke unavailable"
            ),
            "{error}"
        );
    }

    #[tokio::test]
    async fn connect_reports_a_malformed_body() {
        let server = MockServer::start().await;
        Mock::given(path(CONNECTIONS))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ConnectionId": 1}"#))
            .mount(&server)
            .await;

        let error = connect_to_uk1(&client_for(&server)).await.unwrap_err();

        assert!(matches!(error, Error::Decode { .. }), "{error}");
    }

    #[tokio::test]
    async fn disconnect_deletes_the_session_with_only_the_preshared_key() {
        let server = MockServer::start().await;
        with_standard_headers(
            Mock::given(method("DELETE")).and(path(format!("{CONNECTIONS}/48213977"))),
        )
        .and(header("wireguard-psk", PRESHARED))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

        disconnect_from_uk1(&client_for(&server)).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        let headers = &requests[0].headers;
        for name in ["username", "password", "x-gf-agent", "x-gf-realm"] {
            assert!(!headers.contains_key(name), "sent {name}");
        }
        assert!(requests[0].body.is_empty());
    }

    #[tokio::test]
    async fn disconnect_reports_server_errors() {
        let server = MockServer::start().await;
        Mock::given(path(format!("{CONNECTIONS}/48213977")))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let error = disconnect_from_uk1(&client_for(&server)).await.unwrap_err();

        assert!(
            matches!(
                error,
                Error::Http {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    ..
                }
            ),
            "{error}"
        );
    }

    #[tokio::test]
    async fn disconnect_reports_an_unknown_session_as_not_found() {
        let server = MockServer::start().await;
        Mock::given(path(format!("{CONNECTIONS}/48213977")))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let error = disconnect_from_uk1(&client_for(&server)).await.unwrap_err();

        assert!(
            matches!(
                error,
                Error::Http {
                    status: StatusCode::NOT_FOUND,
                    ..
                }
            ),
            "{error}"
        );
    }
}
