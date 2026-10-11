use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::thread;

use ipc::Location;
use tokio::task::JoinSet;

#[derive(Debug, thiserror::Error)]
pub enum FlagError {
    #[error("{0:?} is not a three-letter country code")]
    CountryCode(String),
    #[error("environment variable {0} is not set")]
    MissingEnv(&'static str),
    #[error("accessing {path} failed: {source}")]
    Cache { path: PathBuf, source: io::Error },
    #[error("downloading the flag failed: {0}")]
    Download(#[from] vypr_api::Error),
    #[error("the flag is not a valid PNG: {0}")]
    Decode(#[from] png::DecodingError),
}

/// Downloads each country's flag that is not cached yet, on a background thread. A flag that fails
/// is logged and tried again on the next start.
pub fn cache(locations: &[Location]) -> io::Result<()> {
    let country_codes: BTreeSet<String> = locations
        .iter()
        .map(|location| location.country_code.clone())
        .collect();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    thread::Builder::new()
        .name("flags".into())
        .spawn(move || runtime.block_on(cache_all(country_codes)))?;

    Ok(())
}

async fn cache_all(country_codes: BTreeSet<String>) {
    let api = match vypr_api::Client::new(vypr_api::DEFAULT_BASE_URL) {
        Ok(api) => api,
        Err(error) => return log::warn!("cannot download flags: {error}"),
    };
    let cache_dir = match cache_dir() {
        Ok(cache_dir) => cache_dir,
        Err(error) => return log::warn!("cannot cache flags: {error}"),
    };

    let mut downloads = JoinSet::new();
    for country_code in country_codes {
        let api = api.clone();
        let cache_dir = cache_dir.clone();
        downloads.spawn(async move {
            if let Err(error) = cache_one(&api, &cache_dir, &country_code).await {
                log::warn!("flag {country_code}: {error}");
            }
        });
    }

    while let Some(finished) = downloads.join_next().await {
        if let Err(error) = finished {
            log::error!("caching a flag failed: {error}");
        }
    }
}

/// `%LOCALAPPDATA%\VyprLite\flags` on Windows, `~/Library/Caches/VyprLite/flags` on macOS.
pub fn cache_dir() -> Result<PathBuf, FlagError> {
    let path = caches_root()?.join("VyprLite").join("flags");

    fs::create_dir_all(&path).map_err(|source| FlagError::Cache {
        path: path.clone(),
        source,
    })?;

    Ok(path)
}

#[cfg(windows)]
fn caches_root() -> Result<PathBuf, FlagError> {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or(FlagError::MissingEnv("LOCALAPPDATA"))
}

#[cfg(target_os = "macos")]
fn caches_root() -> Result<PathBuf, FlagError> {
    let home = env::var_os("HOME").ok_or(FlagError::MissingEnv("HOME"))?;

    Ok(PathBuf::from(home).join("Library").join("Caches"))
}

/// The cached file name for `country_code`'s flag, inside `cache_dir()`.
pub fn file_name(country_code: &str) -> Result<String, FlagError> {
    // The code comes from VyprVPN and names a file, so it must not be able to name another path.
    if country_code.len() != 3 || !country_code.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Err(FlagError::CountryCode(country_code.to_owned()));
    }

    Ok(format!("{}.png", country_code.to_ascii_lowercase()))
}

async fn cache_one(
    api: &vypr_api::Client,
    cache_dir: &Path,
    country_code: &str,
) -> Result<(), FlagError> {
    let file_name = file_name(country_code)?;
    let path = cache_dir.join(&file_name);
    if path.exists() {
        return Ok(());
    }

    let png = api.flag(country_code).await?;
    validate(&png)?;

    // Renaming into place is atomic, so a crash mid-write cannot leave a truncated flag that
    // passes for cached.
    let partial = cache_dir.join(format!("{file_name}.part"));
    fs::write(&partial, png).map_err(|source| FlagError::Cache {
        path: partial.clone(),
        source,
    })?;
    fs::rename(&partial, &path).map_err(|source| FlagError::Cache { path, source })
}

/// Decodes the whole image, so a truncated or bogus download is never cached.
fn validate(png: &[u8]) -> Result<(), png::DecodingError> {
    let mut reader = png::Decoder::new(Cursor::new(png)).read_info()?;
    let mut pixels = vec![0; reader.output_buffer_size().unwrap_or_default()];
    reader.next_frame(&mut pixels)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    /// A 2x1 PNG: one opaque red pixel and one transparent pixel.
    fn png_bytes() -> Vec<u8> {
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[255, 0, 0, 255, 0, 0, 0, 0])
            .unwrap();

        png
    }

    #[tokio::test]
    async fn downloads_each_flag_once() {
        let vypr = MockServer::start().await;
        Mock::given(path("/images/vpn_flags/64/ger.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(png_bytes()))
            .expect(1)
            .mount(&vypr)
            .await;
        let api = vypr_api::Client::new(&vypr.uri()).unwrap();
        let cache_dir = tempfile::tempdir().unwrap();

        for _ in 0..2 {
            cache_one(&api, cache_dir.path(), "GER").await.unwrap();
        }
        assert_eq!(
            fs::read(cache_dir.path().join("ger.png")).unwrap(),
            png_bytes()
        );
    }

    #[tokio::test]
    async fn caches_nothing_for_a_missing_or_broken_flag() {
        let vypr = MockServer::start().await;
        Mock::given(path("/images/vpn_flags/64/zzz.png"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&vypr)
            .await;
        Mock::given(path("/images/vpn_flags/64/bad.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(png_bytes()[..30].to_vec()))
            .mount(&vypr)
            .await;
        let api = vypr_api::Client::new(&vypr.uri()).unwrap();
        let cache_dir = tempfile::tempdir().unwrap();

        assert!(matches!(
            cache_one(&api, cache_dir.path(), "ZZZ").await,
            Err(FlagError::Download(_))
        ));
        assert!(matches!(
            cache_one(&api, cache_dir.path(), "BAD").await,
            Err(FlagError::Decode(_))
        ));
        assert_eq!(fs::read_dir(cache_dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn refuses_codes_that_could_name_another_path() {
        let api = vypr_api::Client::new("http://127.0.0.1:9").unwrap();
        let cache_dir = tempfile::tempdir().unwrap();

        for code in ["../", "..\\x", "GERMANY", ""] {
            assert!(
                matches!(
                    cache_one(&api, cache_dir.path(), code).await,
                    Err(FlagError::CountryCode(_))
                ),
                "{code}"
            );
        }
    }
}
