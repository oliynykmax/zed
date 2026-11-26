/// A file of the infoview page, embedded in the binary.
pub struct Asset {
    pub path: &'static str,
    pub mime_type: &'static str,
    pub bytes: &'static [u8],
}

macro_rules! asset {
    ($path:literal, $mime_type:literal) => {
        Asset {
            path: $path,
            mime_type: $mime_type,
            bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/", $path)),
        }
    };
}

const ASSETS: &[Asset] = &[
    asset!("index.html", "text/html; charset=utf-8"),
    asset!("bridge.js", "text/javascript; charset=utf-8"),
    asset!("base.css", "text/css; charset=utf-8"),
    asset!("vendor/index.css", "text/css; charset=utf-8"),
    asset!("vendor/codicon.ttf", "font/ttf"),
    asset!(
        "vendor/index.production.min.js",
        "text/javascript; charset=utf-8"
    ),
    asset!(
        "vendor/loader.production.min.js",
        "text/javascript; charset=utf-8"
    ),
    asset!(
        "vendor/react.production.min.js",
        "text/javascript; charset=utf-8"
    ),
    asset!(
        "vendor/react-dom.production.min.js",
        "text/javascript; charset=utf-8"
    ),
    asset!(
        "vendor/react-jsx-runtime.production.min.js",
        "text/javascript; charset=utf-8"
    ),
];

/// Looks up an asset by its URL path, such as `/vendor/index.css`.
pub fn asset(url_path: &str) -> Option<&'static Asset> {
    let path = url_path.trim_start_matches('/');
    ASSETS.iter().find(|asset| asset.path == path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_page_and_vendored_modules() {
        let page = asset("/index.html").unwrap();
        assert_eq!(page.mime_type, "text/html; charset=utf-8");
        assert!(page.bytes.starts_with(b"<!doctype html>"));

        let infoview = asset("/vendor/index.production.min.js").unwrap();
        assert_eq!(infoview.mime_type, "text/javascript; charset=utf-8");
        assert!(
            infoview
                .bytes
                .windows(b"renderInfoview".len())
                .any(|window| window == b"renderInfoview")
        );

        assert!(asset("/vendor/package.json").is_none());
        assert!(asset("/../Cargo.toml").is_none());
    }
}
