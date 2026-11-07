use std::{env, path::PathBuf};

#[derive(Clone)]
pub struct Config {
    pub base_url: String,
    pub bind: String,
    pub data_dir: PathBuf,
}
impl Config {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let base_url =
            normalize_origin(&env::var("BASE_URL").unwrap_or("http://127.0.0.1:3080".into()))?;
        Ok(Self {
            base_url,
            bind: env::var("BIND").unwrap_or("127.0.0.1:3080".into()),
            data_dir: env::var("DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or("var".into()),
        })
    }
    pub fn secure(&self) -> bool {
        self.base_url.starts_with("https://")
    }
}

fn normalize_origin(input: &str) -> Result<String, Box<dyn std::error::Error>> {
    let url = url::Url::parse(input)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("BASE_URL must be an http(s) origin without a path or credentials".into());
    }
    if url.scheme() == "http"
        && !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
    {
        return Err("Public deployments require HTTPS in BASE_URL".into());
    }
    Ok(url.origin().ascii_serialization())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uppercase_https_keeps_secure_cookie_configuration() {
        let base_url = normalize_origin("HTTPS://EXAMPLE.COM:443/").unwrap();
        let config = Config {
            base_url,
            bind: "127.0.0.1:0".into(),
            data_dir: "var".into(),
        };
        assert!(config.secure());
        assert_eq!(config.base_url, "https://example.com");
    }
    #[test]
    fn origin_rejects_public_http_credentials_and_non_origin_urls() {
        for value in [
            "http://example.com",
            "https://user:secret@example.com",
            "https://example.com/path",
            "https://example.com/?q=1",
            "https://example.com/#part",
            "file:///tmp/file",
        ] {
            assert!(normalize_origin(value).is_err(), "{value}");
        }
        assert_eq!(
            normalize_origin("http://[::1]:3080/").unwrap(),
            "http://[::1]:3080"
        );
    }
}
