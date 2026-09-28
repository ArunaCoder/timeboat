//! A guarda de navegação da janela: o webview não sai das páginas do próprio app.
//!
//! A CSP governa o que a página carrega, e não para onde a janela vai. Sem esta guarda, um
//! link ou script que levasse a janela a uma página de fora a exibiria com o título e o
//! ícone do app — e com a ponte IPC ao alcance dela.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager as _, Runtime, Url};

/// O nome do plugin, como o Tauri o registra.
const PLUGIN_NAME: &str = "navigation-guard";

/// O plugin que recusa toda navegação para fora das páginas do app.
///
/// A origem de desenvolvimento só vale no build de `tauri dev` (`cfg(dev)`): no instalado,
/// um servidor qualquer ouvindo na porta do `devUrl` não é o app.
#[must_use]
pub fn guard<R: Runtime>() -> TauriPlugin<R> {
    Builder::new(PLUGIN_NAME)
        .on_navigation(|webview, url| {
            let config = webview.app_handle().config();
            let dev_url = if cfg!(dev) { config.build.dev_url.as_ref() } else { None };
            let allowed = is_app_url(url, dev_url);
            if !allowed {
                // Só a origem: o resto da URL poderia carregar dado da página.
                log::warn!(
                    "blocked the {} webview from navigating away from the app, to {}",
                    webview.label(),
                    url.origin().ascii_serialization()
                );
            }
            allowed
        })
        .build()
}

/// Se `url` é uma página do app: a empacotada, servida pelo protocolo do Tauri no Windows
/// (`http://tauri.localhost`), ou a do servidor de desenvolvimento, comparada pela origem.
fn is_app_url(url: &Url, dev_url: Option<&Url>) -> bool {
    let packaged = matches!(url.scheme(), "http" | "https") && url.host_str() == Some("tauri.localhost");
    let development = dev_url.is_some_and(|dev| dev.origin() == url.origin());
    packaged || development
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use tauri::Url;

    use super::is_app_url;

    type TestResult = Result<(), Box<dyn Error>>;

    /// A página empacotada passa; a de fora, e a que só carrega o host do app no caminho ou
    /// no começo de outro host, não.
    #[test]
    fn only_the_pages_of_the_app_are_allowed() -> TestResult {
        assert!(is_app_url(&Url::parse("http://tauri.localhost/index.html")?, None));
        for url in [
            "https://evil.example/?d=x",
            "http://tauri.localhost.evil.example/",
            "http://evil.example/tauri.localhost",
            "file:///C:/Windows/win.ini",
        ] {
            assert!(!is_app_url(&Url::parse(url)?, None), "{url} não é página do app");
        }
        Ok(())
    }

    /// A origem de desenvolvimento vale inteira — esquema, host e porta —, e só ela.
    #[test]
    fn the_dev_origin_is_compared_whole() -> TestResult {
        let dev = Url::parse("http://localhost:1430")?;
        assert!(is_app_url(
            &Url::parse("http://localhost:1430/src/main.ts")?,
            Some(&dev)
        ));
        assert!(!is_app_url(&Url::parse("http://localhost:1431/")?, Some(&dev)));
        assert!(!is_app_url(&Url::parse("http://localhost:1430/")?, None));
        Ok(())
    }
}
