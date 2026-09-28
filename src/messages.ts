// As frases que a pessoa lê: a de cada código de erro e a de cada desfecho.
//
// O `message` que vem do Rust é diagnóstico em inglês e nunca é a frase principal: ele só
// aparece como "detalhe técnico", que ajuda quem for investigar uma falha do ffmpeg.

import { formatClock, wholeSeconds } from "./durations.ts";
import type { IpcErrorCode } from "./ipc-error-codes.generated.ts";
import type { IpcError, JobOutcome } from "./types.ts";

/// A frase de cada código. O `Record` cobra a frase de todo código novo na compilação.
export const ERROR_MESSAGES: Readonly<Record<IpcErrorCode, string>> = {
  ffmpeg_missing:
    "O ffmpeg não foi encontrado. Instale-o com “winget install Gyan.FFmpeg” e abra o Timeboat de novo.",
  ffmpeg_failed: "O ffmpeg não conseguiu processar este arquivo.",
  unsupported_file:
    "Este arquivo não é um vídeo ou áudio que o Timeboat aceite.",
  multiple_files: "Solte um arquivo por vez.",
  busy: "Já há um arquivo sendo processado. Espere terminar ou cancele.",
  no_audio:
    "Este arquivo não tem trilha de áudio, então não há silêncio para detectar.",
  unreadable_media:
    "Não foi possível ler este arquivo. Ele pode estar corrompido ou incompleto.",
  output_folder:
    "Não foi possível gravar o resultado na pasta do arquivo. Confira se a pasta aceita gravação e se há espaço em disco.",
  temp_file:
    "Não foi possível criar um arquivo temporário para o processamento. Confira o espaço em disco.",
  invalid_settings: "Alguma configuração está fora da faixa aceita.",
  settings_store: "Não foi possível salvar as configurações.",
  no_output: "Ainda não há arquivo gerado para mostrar.",
  reveal: "Não foi possível abrir a pasta do arquivo.",
};

/// A frase de quando a falha não veio da fronteira IPC (um bug, não um desfecho previsto).
const UNEXPECTED = "Algo deu errado de um jeito inesperado.";

/// Se `value` é o erro serializado do Rust, com um código que esta versão conhece.
export function isIpcError(value: unknown): value is IpcError {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  if (!("code" in value) || !("message" in value)) {
    return false;
  }
  const { code, message } = value;
  return (
    typeof code === "string" &&
    typeof message === "string" &&
    Object.hasOwn(ERROR_MESSAGES, code)
  );
}

/// O que se mostra de uma falha: a frase e, quando houver, o diagnóstico técnico.
export interface ErrorText {
  readonly text: string;
  readonly detail: string | null;
}

/// A frase de qualquer falha — do Rust ou de um bug do próprio webview.
export function describeError(error: unknown): ErrorText {
  if (isIpcError(error)) {
    return { text: ERROR_MESSAGES[error.code], detail: error.message };
  }
  if (error instanceof Error) {
    return { text: UNEXPECTED, detail: error.message };
  }
  return { text: UNEXPECTED, detail: typeof error === "string" ? error : null };
}

/// O tom visual de um desfecho.
export type Tone = "success" | "info" | "error";

/// Um número do resumo, com o rótulo que o explica.
export interface Stat {
  readonly label: string;
  readonly value: string;
  /// Os números que a pessoa procura primeiro, em destaque.
  readonly highlight: boolean;
}

/// Como a tela apresenta um desfecho.
export interface OutcomeText {
  readonly tone: Tone;
  readonly title: string;
  readonly lines: readonly string[];
  readonly stats: readonly Stat[];
  readonly detail: string | null;
  /// Se há resultado gravado para mostrar no Explorer.
  readonly canReveal: boolean;
}

/// O tempo de processamento. Abaixo de um segundo o relógio diria "0:00", como se nada
/// tivesse acontecido.
function formatElapsed(secs: number): string {
  return secs < 1 ? "menos de 1 segundo" : formatClock(secs);
}

/// O resumo do resultado gravado: quanto o arquivo perdeu e quanto o processamento levou
/// primeiro, e depois o detalhe.
///
/// A perda é a diferença das durações **já arredondadas** ao segundo, e não a arredondada
/// da diferença: é o que faz "12:05 → 10:01" e "perdeu 2:04" fecharem a conta na tela.
function finishedStats(report: FinishedReport): readonly Stat[] {
  const original = wholeSeconds(report.original_secs);
  const final = wholeSeconds(report.final_secs);
  return [
    {
      label: report.video ? "O vídeo perdeu" : "O áudio perdeu",
      value: formatClock(original - final),
      highlight: true,
    },
    {
      label: "Tempo de processamento",
      value: formatElapsed(report.elapsed_secs),
      highlight: true,
    },
    {
      label: "Duração",
      value: `${formatClock(original)} → ${formatClock(final)}`,
      highlight: false,
    },
    {
      label: "Silêncios removidos",
      value: String(report.cut_count),
      highlight: false,
    },
  ];
}

type FinishedReport = Extract<JobOutcome, { kind: "finished" }>["report"];

/// A apresentação de cada desfecho de um processamento.
export function describeOutcome(outcome: JobOutcome): OutcomeText {
  switch (outcome.kind) {
    case "finished":
      return {
        tone: "success",
        title: "Pronto!",
        lines: [
          `Salvo como “${outcome.report.output_name}”, na mesma pasta do original.`,
        ],
        stats: finishedStats(outcome.report),
        detail: null,
        canReveal: true,
      };
    case "nothing_to_remove":
      return {
        tone: "info",
        title: "Nenhum silêncio para remover",
        lines: [
          "Nenhum trecho de silêncio passou das configurações atuais.",
          "Nenhum arquivo foi gerado.",
        ],
        stats: [],
        detail: null,
        canReveal: false,
      };
    case "all_silent":
      return {
        tone: "info",
        title: "O arquivo inteiro ficou abaixo do nível de som",
        lines: [
          "Todo o áudio conta como silêncio, e não sobraria nada. Nenhum arquivo foi gerado.",
          "Experimente um nível de som mais baixo, como -50 dB.",
        ],
        stats: [],
        detail: null,
        canReveal: false,
      };
    case "cancelled":
      return {
        tone: "info",
        title: "Processamento interrompido",
        lines: [
          "O arquivo original não foi alterado, e nenhum arquivo incompleto ficou na pasta.",
        ],
        stats: [],
        detail: null,
        canReveal: false,
      };
    case "failed": {
      const { text, detail } = describeError(outcome.error);
      return {
        tone: "error",
        title: "Não deu certo",
        lines: [text, "O arquivo original não foi alterado."],
        stats: [],
        detail,
        canReveal: false,
      };
    }
  }
}
