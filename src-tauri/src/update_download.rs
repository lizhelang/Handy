//! 只下载发布文档：HTTPS、固定起点、显式重定向来源、限时和有界流读取。
use inputia_release::catalog::{CheckError, CheckResult, DocumentSource, EmbeddedCatalog};
use reqwest::{redirect::Policy, Client, Url};
use std::{collections::BTreeSet, time::Duration};

pub struct HttpsCatalogSource {
    client: Client,
    base: Url,
}
fn secure_url(raw: &str) -> CheckResult<Url> {
    let url = Url::parse(raw).map_err(|_| CheckError::Network)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default() != Some(443)
        || raw.contains(['\\', '%'])
    {
        return Err(CheckError::Network);
    }
    Ok(url)
}
impl HttpsCatalogSource {
    pub fn new(config: &EmbeddedCatalog) -> CheckResult<Self> {
        let base = secure_url(&config.base_url)?;
        if !config.base_url.ends_with('/') || config.redirect_origins.len() > 8 {
            return Err(CheckError::Network);
        }
        let mut origins = BTreeSet::from([base.origin().ascii_serialization()]);
        for raw in &config.redirect_origins {
            let url = secure_url(raw)?;
            if url.path() != "/" {
                return Err(CheckError::Network);
            }
            origins.insert(url.origin().ascii_serialization());
        }
        let client = Client::builder()
            .https_only(true)
            .referer(false)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .redirect(Policy::custom(move |attempt| {
                let url = attempt.url();
                if attempt.previous().len() > 3
                    || url.scheme() != "https"
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || !origins.contains(&url.origin().ascii_serialization())
                {
                    attempt.error("untrusted release redirect")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|_| CheckError::Network)?;
        Ok(Self { client, base })
    }
    fn url(&self, relative: &str) -> CheckResult<Url> {
        inputia_release::safe_relative(relative)?;
        if relative.contains(['%', '?', '#']) {
            return Err(CheckError::Network);
        }
        let url = self.base.join(relative).map_err(|_| CheckError::Network)?;
        if url.origin() != self.base.origin() || !url.path().starts_with(self.base.path()) {
            return Err(CheckError::Network);
        }
        Ok(url)
    }
}
impl DocumentSource for HttpsCatalogSource {
    async fn fetch(&self, relative: &str, limit: usize) -> CheckResult<Vec<u8>> {
        let mut response = self
            .client
            .get(self.url(relative)?)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|_| CheckError::Network)?;
        if response.status() != reqwest::StatusCode::OK
            || response.headers().contains_key("content-encoding")
        {
            return Err(CheckError::Network);
        }
        if response
            .content_length()
            .is_some_and(|size| size > limit as u64)
        {
            return Err(CheckError::BudgetExceeded);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| CheckError::Network)? {
            if chunk.len() > limit.saturating_sub(bytes.len()) {
                return Err(CheckError::BudgetExceeded);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origins_and_paths_never_escape_embedded_catalog() {
        for url in [
            "http://updates.example/",
            "https://user@updates.example/",
            "https://updates.example:8443/",
            "https://updates.example/?token=abc",
            "https://updates.example/%2e%2e/",
        ] {
            assert!(secure_url(url).is_err());
        }
        let source = HttpsCatalogSource {
            client: Client::new(),
            base: secure_url("https://updates.example/inputia/").unwrap(),
        };
        for path in [
            "../evil",
            "/evil",
            "//evil.example/a",
            "a%2fb",
            "a?b",
            "a#b",
            "a/../b",
        ] {
            assert!(source.url(path).is_err());
        }
        assert_eq!(
            source
                .url("channels/stable/macos/arm64.json")
                .unwrap()
                .as_str(),
            "https://updates.example/inputia/channels/stable/macos/arm64.json"
        );
    }
}
