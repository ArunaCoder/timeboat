//! Binário Tauri: só delega para a lib (`timeboat_lib::run`) — toda a lógica da casca
//! vive em `lib.rs`, mantendo este arquivo mínimo (padrão Tauri).

// Impede a janela de console extra no Windows em release — NÃO REMOVER.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    timeboat_lib::run();
}
