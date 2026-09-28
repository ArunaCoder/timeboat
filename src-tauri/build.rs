use tauri_build::{AppManifest, Attributes};

/// Os comandos do app, pelo nome com que o webview os chama — o manifesto que faz do ACL do
/// Tauri a lista mínima de permissões.
///
/// Sem manifesto, todo comando registrado responde a **qualquer** janela, e a capability só
/// restringe plugins. Com ele, o `tauri-build` gera um `allow-<comando>` por nome (o `_` vira
/// `-`), e a janela só alcança o comando que `capabilities/default.json` concede. Comando novo
/// entra aqui, no `generate_handler!` de `src/lib.rs` e na capability; quem confere que as três
/// listas dizem os mesmos nomes é o teste `ipc::tests::the_three_command_lists_agree`.
const COMMANDS: &[&str] = &[
    "get_settings",
    "save_settings",
    "reset_settings",
    "check_tools",
    "pick_media",
    "cancel_job",
    "current_job",
    "reveal_output",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tauri_build::try_build(Attributes::new().app_manifest(AppManifest::new().commands(COMMANDS)))?;
    Ok(())
}
