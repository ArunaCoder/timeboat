import { describe, expect, it } from "vitest";

import {
  describeError,
  describeOutcome,
  ERROR_MESSAGES,
  isIpcError,
} from "./messages.ts";

describe("isIpcError", () => {
  it("reconhece o erro serializado do Rust", () => {
    expect(isIpcError({ code: "busy", message: "a file is busy" })).toBe(true);
  });

  it("recusa código desconhecido e forma errada", () => {
    for (const value of [
      { code: "nope", message: "x" },
      { code: "busy" },
      { code: 1, message: "x" },
      "busy",
      null,
      undefined,
    ]) {
      expect(isIpcError(value)).toBe(false);
    }
  });

  it("não confunde propriedade herdada com código", () => {
    expect(isIpcError({ code: "toString", message: "x" })).toBe(false);
  });
});

describe("describeError", () => {
  it("usa a frase do código e guarda o diagnóstico como detalhe", () => {
    expect(
      describeError({ code: "ffmpeg_missing", message: "ffmpeg not found" }),
    ).toEqual({
      text: ERROR_MESSAGES.ffmpeg_missing,
      detail: "ffmpeg not found",
    });
  });

  it("não exibe o texto cru de um erro inesperado como frase", () => {
    const described = describeError(new Error("boom"));
    expect(described.text).not.toContain("boom");
    expect(described.detail).toBe("boom");
    expect(describeError(42).detail).toBeNull();
  });
});

describe("describeOutcome", () => {
  const report = {
    output_name: "aula (TIMEBOATED).mp4",
    original_secs: 725.2,
    final_secs: 600.4,
    cut_count: 14,
    video: true,
    elapsed_secs: 42.4,
  };

  it("resume o resultado com o tempo perdido e o de processamento em destaque", () => {
    const text = describeOutcome({ kind: "finished", report });
    expect(text.tone).toBe("success");
    expect(text.canReveal).toBe(true);
    expect(text.lines).toEqual([
      "Salvo como “aula (TIMEBOATED).mp4”, na mesma pasta do original.",
    ]);
    expect(text.stats).toEqual([
      { label: "O vídeo perdeu", value: "2:05", highlight: true },
      { label: "Tempo de processamento", value: "0:42", highlight: true },
      { label: "Duração", value: "12:05 → 10:00", highlight: false },
      { label: "Silêncios removidos", value: "14", highlight: false },
    ]);
  });

  /// 12:05,4 → 10:00,6: a diferença exata (2:04,8) arredondaria a 2:05, mas a tela mostra
  /// 12:05 → 10:01, e a conta que a pessoa faz de cabeça dá 2:04.
  it("faz a perda fechar com as durações mostradas", () => {
    const text = describeOutcome({
      kind: "finished",
      report: { ...report, original_secs: 725.4, final_secs: 600.6 },
    });
    expect(text.stats[0]?.value).toBe("2:04");
    expect(text.stats[2]?.value).toBe("12:05 → 10:01");
  });

  it("fala do áudio quando o resultado não tem vídeo", () => {
    const text = describeOutcome({
      kind: "finished",
      report: { ...report, video: false },
    });
    expect(text.stats[0]?.label).toBe("O áudio perdeu");
  });

  it("não mostra 0:00 para um processamento de menos de um segundo", () => {
    const text = describeOutcome({
      kind: "finished",
      report: { ...report, elapsed_secs: 0.4 },
    });
    expect(text.stats[1]?.value).toBe("menos de 1 segundo");
  });

  it("garante, na interrupção, que o original ficou intacto", () => {
    const text = describeOutcome({ kind: "cancelled" });
    expect(text.stats).toEqual([]);
    expect(text.lines.join(" ")).toContain("original não foi alterado");
  });

  it("só oferece o Explorer quando há arquivo gravado", () => {
    for (const kind of [
      "nothing_to_remove",
      "all_silent",
      "cancelled",
    ] as const) {
      expect(describeOutcome({ kind }).canReveal).toBe(false);
    }
  });

  it("mostra a frase da falha e o diagnóstico à parte", () => {
    const text = describeOutcome({
      kind: "failed",
      error: { code: "no_audio", message: "the media has no audio stream" },
    });
    expect(text.tone).toBe("error");
    expect(text.lines[0]).toBe(ERROR_MESSAGES.no_audio);
    expect(text.detail).toBe("the media has no audio stream");
  });
});
