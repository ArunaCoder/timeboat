//! Timeboat: remove os silêncios de um vídeo ou áudio e grava o resultado na mesma pasta.
//!
//! A pessoa solta o arquivo na janela (ou o escolhe no seletor), e o app faz o resto com o
//! `ffmpeg` instalado na máquina: mede a duração ([`probe`]), acha os silêncios
//! ([`silence`]), decide o que cortar pelas quatro réguas editáveis ([`settings`],
//! [`timeline`]) e grava os trechos mantidos num arquivo novo ([`render`], [`media`]). O
//! caminho inteiro está em [`pipeline`], sem Tauri; a ponte com a janela é o [`job`].
//!
//! # O que ele leva a sério
//!
//! - **Nunca perde nem sobrescreve arquivo.** O original não é tocado; o resultado ganha
//!   nome próprio, reservado antes de o `ffmpeg` começar, e o que for interrompido não deixa
//!   rastro na pasta.
//! - **O webview não manda em arquivo nenhum.** Caminhos não atravessam a fronteira IPC.
//! - **Toda falha tem nome.** Cada desfecho tem código próprio ([`error::IpcErrorCode`]) e
//!   frase em português na tela.

pub mod error;
pub mod ipc;
pub mod job;
pub mod media;
pub mod navigation;
pub mod pipeline;
pub mod probe;
pub mod render;
pub mod settings;
pub mod silence;
pub mod timeline;
pub mod tool;

#[cfg(test)]
mod test_support;

use std::time::Duration;

use tauri::{DragDropEvent, Manager as _, RunEvent, WindowEvent};

use crate::job::Jobs;
use crate::settings::SettingsStore;

/// O estado do app, gerenciado pelo Tauri e visto pelos comandos.
#[derive(Debug)]
pub struct AppState {
    /// As réguas da detecção e o arquivo que as persiste.
    pub settings: SettingsStore,
    /// O processamento em andamento e o último resultado.
    pub jobs: Jobs,
}

/// Quanto o fechamento do app espera a limpeza de um processamento interrompido.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Sobe a janela e registra os comandos.
pub fn run() {
    let built = tauri::Builder::default()
        // Nível fixado aqui: o padrão do plugin é `Trace`, e o arquivo de log na máquina da
        // pessoa passaria a ser o rastro das dependências em vez do diagnóstico deste app.
        .plugin(tauri_plugin_log::Builder::new().level(log::LevelFilter::Info).build())
        .plugin(navigation::guard())
        // O seletor de arquivos, aberto só pelo Rust (`ipc::pick_media`).
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let settings_path = app.path().app_config_dir()?.join(settings::FILE_NAME);
            app.manage(AppState {
                settings: SettingsStore::load(settings_path),
                jobs: Jobs::default(),
            });
            Ok(())
        })
        // O soltar nativo: o caminho chega aqui, e não ao webview (vide `job`).
        .on_window_event(|window, event| {
            if let WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) = event {
                job::start(window.app_handle(), paths);
            }
        })
        .invoke_handler(tauri::generate_handler![
            ipc::get_settings,
            ipc::save_settings,
            ipc::reset_settings,
            ipc::check_tools,
            ipc::pick_media,
            ipc::cancel_job,
            ipc::current_job,
            ipc::reveal_output,
        ])
        .build(tauri::generate_context!());
    match built {
        Ok(app) => app.run(|handle, event| {
            // Fechar no meio de um processamento interrompe o `ffmpeg` e espera a limpeza do
            // parcial; sem isso o processo filho sobreviveria ao app.
            if matches!(event, RunEvent::Exit)
                && let Some(state) = handle.try_state::<AppState>()
            {
                state.jobs.shutdown(SHUTDOWN_GRACE);
            }
        }),
        Err(error) => {
            // Sem janela não há tela para avisar. O `eprintln!` cobre a falha que antecede o
            // logger do plugin (WebView2 ausente, contexto que não sobe).
            eprintln!("failed to start timeboat: {error}");
            log::error!("failed to start timeboat: {error}");
        }
    }
}
