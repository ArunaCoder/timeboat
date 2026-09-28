import { describe, expect, it } from "vitest";

import { checkField, FIELD_ORDER } from "./settings-fields.ts";
import type { Settings } from "./types.ts";

describe("FIELD_ORDER", () => {
  it("cobre cada régua uma vez", () => {
    const sample: Settings = {
      threshold_db: -40,
      min_silence_secs: 3,
      min_sound_secs: 0.75,
      padding_secs: 0.25,
    };
    expect([...FIELD_ORDER].sort()).toEqual(Object.keys(sample).sort());
  });
});

describe("checkField", () => {
  const bounds = { min: -100, max: -1 };

  it("devolve o número dentro da faixa, inclusive nas pontas", () => {
    expect(checkField("-40,5", bounds)).toEqual({ ok: true, value: -40.5 });
    expect(checkField("-100", bounds)).toEqual({ ok: true, value: -100 });
    expect(checkField("-1", bounds)).toEqual({ ok: true, value: -1 });
  });

  it("arredonda às três casas que o campo exibe, antes de conferir a faixa", () => {
    expect(checkField("-40,12345", bounds)).toEqual({
      ok: true,
      value: -40.123,
    });
    expect(checkField("-0,9996", bounds)).toEqual({ ok: true, value: -1 });
    // O `toEqual` compara com `Object.is`: o zero negativo não passaria por zero.
    expect(checkField("-0", { min: 0, max: 1 })).toEqual({
      ok: true,
      value: 0,
    });
  });

  it("diz a faixa quando o número está fora dela", () => {
    expect(checkField("0", bounds)).toEqual({
      ok: false,
      reason: "Use um valor entre -100 e -1.",
    });
  });

  it("pede um número quando o texto não é um", () => {
    const check = checkField("quarenta", bounds);
    expect(check.ok).toBe(false);
  });
});
