// Arquivo GERADO — não editar à mão.
//
// Fonte: o enum `IpcErrorCode` de `src-tauri/src/error.rs`. Quem o produz e confere é o
// teste `error::tests::generated_ts_matches_the_enum`, que falha enquanto o arquivo
// commitado divergir do que o Rust produz hoje. Para reescrevê-lo depois de mexer na
// fonte, rode a suíte com `UPDATE_IPC_TS=1`.

/// Código de erro estável da fronteira IPC — um literal por variante do enum do Rust, na
/// forma que o serde serializa.
export type IpcErrorCode =
  | "ffmpeg_missing"
  | "ffmpeg_failed"
  | "unsupported_file"
  | "multiple_files"
  | "busy"
  | "no_audio"
  | "unreadable_media"
  | "output_folder"
  | "temp_file"
  | "invalid_settings"
  | "settings_store"
  | "no_output"
  | "reveal"
  | "internal";
