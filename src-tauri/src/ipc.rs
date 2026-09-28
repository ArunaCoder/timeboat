//! Os comandos que o webview invoca.
//!
//! Nenhum recebe caminho de arquivo: o arquivo a processar chega pelo soltar na janela ou
//! pelo seletor que [`pick_media`] abre, os dois vistos só pelo Rust (vide [`crate::job`]).
//!
//! Os comandos que tocam disco ou iniciam processo são `#[tauri::command(async)]`: no Tauri
//! v2 o comando síncrono roda na thread principal, e a janela congelaria durante a espera.
//!
//! Os parâmetros que o Tauri exige por valor (`AppHandle`, `State`, a janela) são religados
//! a um nome local na primeira linha: é o idioma que cala o `needless_pass_by_value` sem
//! afrouxar o lint para o arquivo inteiro.

use serde::Serialize;
use tauri::{AppHandle, State, WebviewWindow};
use tauri_plugin_dialog::{DialogExt as _, FilePath};

use crate::AppState;
use crate::error::{AppError, IpcError};
use crate::job::{self, JobSnapshot};
use crate::media;
use crate::settings::{DEFAULTS, LIMITS, Limits, Settings};
use crate::tool;

/// As configurações correntes, os padrões de fábrica e as faixas aceitas — tudo o que o
/// formulário precisa, de uma vez.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SettingsView {
    /// As réguas em vigor.
    pub current: Settings,
    /// Os padrões de fábrica.
    pub defaults: Settings,
    /// A faixa aceita de cada régua.
    pub limits: Limits,
}

/// As configurações correntes, com os padrões e as faixas.
#[tauri::command]
#[must_use]
pub fn get_settings(managed: State<'_, AppState>) -> SettingsView {
    let state = managed;
    SettingsView {
        current: state.settings.current(),
        defaults: DEFAULTS,
        limits: LIMITS,
    }
}

/// Valida e grava as réguas; devolve as que ficaram em vigor.
///
/// # Errors
/// `invalid_settings` se alguma régua estiver fora da faixa; `settings_store` se o arquivo
/// não puder ser gravado.
#[tauri::command(async)]
pub fn save_settings(managed: State<'_, AppState>, settings: Settings) -> Result<Settings, IpcError> {
    let state = managed;
    state.settings.save(settings).map_err(IpcError::from)
}

/// Volta aos padrões de fábrica e os devolve.
///
/// # Errors
/// `settings_store` se o arquivo não puder ser gravado.
#[tauri::command(async)]
pub fn reset_settings(managed: State<'_, AppState>) -> Result<Settings, IpcError> {
    let state = managed;
    state.settings.save(DEFAULTS).map_err(IpcError::from)
}

/// Confere que o `ffmpeg` e o `ffprobe` estão instalados, para a janela avisar antes do
/// primeiro arquivo.
///
/// # Errors
/// `ffmpeg_missing` se algum faltar no `PATH`; `ffmpeg_failed` se não responder.
#[tauri::command(async)]
pub fn check_tools() -> Result<(), IpcError> {
    tool::check_tools().map_err(IpcError::from)
}

/// Abre o seletor de arquivos nativo; o arquivo escolhido começa a ser processado, e o
/// andamento chega pelos eventos de [`job::EVENT`].
#[tauri::command]
pub fn pick_media(app: AppHandle, window: WebviewWindow) {
    let handle = app;
    let parent = window;
    let extensions = media::accepted_extensions();
    let callback_handle = handle.clone();
    handle
        .dialog()
        .file()
        .set_title("Escolha um vídeo ou áudio")
        .set_parent(&parent)
        .add_filter("Vídeos e áudios", &extensions)
        .pick_file(move |picked| on_picked(&callback_handle, picked));
}

fn on_picked(app: &AppHandle, picked: Option<FilePath>) {
    let Some(picked) = picked else {
        return;
    };
    match picked.into_path() {
        Ok(path) => job::start(app, &[path]),
        Err(error) => log::warn!("the file dialog returned an unusable path: {error}"),
    }
}

/// Pede a interrupção do processamento em andamento; o desfecho chega como evento.
#[tauri::command]
pub fn cancel_job(managed: State<'_, AppState>) {
    let state = managed;
    state.jobs.cancel();
}

/// O processamento em andamento, se há um — para a janela que (re)abre no meio dele retomar
/// o nome do arquivo e o cronômetro.
#[tauri::command]
#[must_use]
pub fn current_job(managed: State<'_, AppState>) -> Option<JobSnapshot> {
    let state = managed;
    state.jobs.current()
}

/// Abre o Explorer na pasta do último resultado, com ele selecionado.
///
/// # Errors
/// `no_output` se nada foi gravado nesta sessão; `reveal` se o Explorer não abrir.
#[tauri::command(async)]
pub fn reveal_output(managed: State<'_, AppState>) -> Result<(), IpcError> {
    let state = managed;
    let output = state.jobs.last_output().ok_or(AppError::NoOutput)?;
    tauri_plugin_opener::reveal_item_in_dir(&output)
        .map_err(|error| IpcError::from(AppError::Reveal(error.to_string())))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::error::Error;
    use std::fs;
    use std::path::Path;

    type TestResult = Result<(), Box<dyn Error>>;

    /// O trecho de `text` entre a primeira ocorrência de `from` e o primeiro `to` depois dela.
    fn section<'a>(text: &'a str, from: &str, to: &str) -> Result<&'a str, Box<dyn Error>> {
        let (_, after) = text.split_once(from).ok_or_else(|| format!("{from} não encontrado"))?;
        let (inside, _) = after.split_once(to).ok_or_else(|| format!("{to} não encontrado"))?;
        Ok(inside)
    }

    /// As três listas de comandos dizem os mesmos nomes: o manifesto do `build.rs`, que gera
    /// as permissões; o `generate_handler!` do `lib.rs`, que registra os comandos; e a
    /// capability da janela, que os concede. Comando registrado sem permissão falha em
    /// runtime com "not allowed", e permissão sem comando é superfície morta.
    #[test]
    fn the_three_command_lists_agree() -> TestResult {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));

        let build = fs::read_to_string(root.join("build.rs"))?;
        let manifest: BTreeSet<String> = section(&build, "const COMMANDS", "];")?
            .split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect();

        let lib = fs::read_to_string(root.join("src").join("lib.rs"))?;
        let handler: BTreeSet<String> = section(&lib, "generate_handler![", "]")?
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(|entry| entry.trim_start_matches("ipc::").to_owned())
            .collect();

        let capability: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join("capabilities").join("default.json"))?)?;
        let granted: BTreeSet<String> = capability["permissions"]
            .as_array()
            .ok_or("a capability não tem lista de permissões")?
            .iter()
            .filter_map(serde_json::Value::as_str)
            .filter_map(|permission| permission.strip_prefix("allow-"))
            .map(|command| command.replace('-', "_"))
            .collect();

        assert!(!manifest.is_empty(), "o manifesto não pode sair vazio da leitura");
        assert_eq!(manifest, handler, "build.rs × generate_handler!");
        assert_eq!(manifest, granted, "build.rs × capabilities/default.json");
        Ok(())
    }
}
