// Espelho dos tipos que cruzam a fronteira IPC. Cada interface corresponde a um tipo
// `Serialize` de `src-tauri/src/`, e os nomes de campo são os do Rust (snake_case) — o
// serde não os renomeia.
//
// Espelhado **à mão**, com uma exceção: o union `IpcErrorCode` é **gerado** do enum do
// Rust (vide `ipc-error-codes.generated.ts`). A forma dos eventos de processamento é presa
// do lado do Rust pelo teste `job::tests::events_keep_their_wire_shape`; campo novo lá que
// não ganhe linha aqui chega em runtime como uma chave que ninguém lê.

import type { IpcErrorCode } from "./ipc-error-codes.generated.ts";

/// Erro de comando ou de processamento. Espelha `error::IpcError`.
export interface IpcError {
  readonly code: IpcErrorCode;
  /// Diagnóstico em inglês — a tela o mostra só como detalhe técnico.
  readonly message: string;
}

/// As quatro réguas da detecção. Espelha `settings::Settings`.
export interface Settings {
  /// Nível, em dBFS, abaixo do qual o áudio conta como silêncio.
  readonly threshold_db: number;
  /// Só o silêncio mais longo que isto, em segundos, é removido.
  readonly min_silence_secs: number;
  /// O som mais curto que isto, em segundos, é ignorado.
  readonly min_sound_secs: number;
  /// Silêncio preservado, em segundos, de cada lado de cada corte.
  readonly padding_secs: number;
}

/// O nome de uma régua.
export type SettingKey = keyof Settings;

/// Faixa fechada aceita por uma régua. Espelha `settings::Bounds`.
export interface Bounds {
  readonly min: number;
  readonly max: number;
}

/// A faixa aceita de cada régua. Espelha `settings::Limits`.
export type Limits = { readonly [Key in SettingKey]: Bounds };

/// O que o formulário recebe na abertura. Espelha `ipc::SettingsView`.
export interface SettingsView {
  readonly current: Settings;
  readonly defaults: Settings;
  readonly limits: Limits;
}

/// A etapa em andamento. Espelha `pipeline::Phase`.
export type Phase = "analyzing" | "rendering";

/// O resumo do resultado gravado. Espelha `job::Report`.
export interface Report {
  readonly output_name: string;
  readonly original_secs: number;
  readonly final_secs: number;
  readonly cut_count: number;
  /// Se o resultado é vídeo (ou só áudio).
  readonly video: boolean;
  /// Quanto o processamento levou, medido pelo Rust.
  readonly elapsed_secs: number;
}

/// O processamento em andamento. Espelha `job::JobSnapshot`.
export interface JobSnapshot {
  readonly file_name: string;
  readonly elapsed_secs: number;
}

/// O que a janela fica sabendo de um processamento. Espelha `job::JobEvent`.
export type JobEvent =
  | { readonly kind: "started"; readonly file_name: string }
  | {
      readonly kind: "progress";
      readonly phase: Phase;
      /// A fração cumprida do processamento inteiro — a barra é uma só.
      readonly fraction: number;
    }
  | { readonly kind: "finished"; readonly report: Report }
  | { readonly kind: "nothing_to_remove" }
  | { readonly kind: "all_silent" }
  | { readonly kind: "cancelled" }
  | { readonly kind: "failed"; readonly error: IpcError }
  | { readonly kind: "rejected"; readonly error: IpcError };

/// Os desfechos de um processamento — o que encerra o andamento.
export type JobOutcome = Extract<
  JobEvent,
  {
    readonly kind:
      | "finished"
      | "nothing_to_remove"
      | "all_silent"
      | "cancelled"
      | "failed";
  }
>;
