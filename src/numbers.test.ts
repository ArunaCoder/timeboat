import { describe, expect, it } from "vitest";

import { formatDecimal, parseDecimal } from "./numbers.ts";

describe("parseDecimal", () => {
  it("aceita vírgula ou ponto como separador decimal", () => {
    expect(parseDecimal("0,75")).toBe(0.75);
    expect(parseDecimal("0.75")).toBe(0.75);
    expect(parseDecimal(",5")).toBe(0.5);
    expect(parseDecimal("3,")).toBe(3);
  });

  it("aceita o sinal comum, o tipográfico e espaços em volta", () => {
    expect(parseDecimal("-40,000")).toBe(-40);
    expect(parseDecimal(" −40 ")).toBe(-40);
    expect(parseDecimal("+2")).toBe(2);
  });

  it("recusa o que não é um número decimal simples", () => {
    for (const text of [
      "",
      "-",
      ",",
      "1,2,3",
      "1.000,5",
      "abc",
      "1e3",
      "3 s",
    ]) {
      expect(parseDecimal(text), text).toBeNull();
    }
  });
});

describe("formatDecimal", () => {
  it("usa vírgula e descarta zeros sobrando", () => {
    expect(formatDecimal(-40)).toBe("-40");
    expect(formatDecimal(0.75)).toBe("0,75");
    expect(formatDecimal(0.1234)).toBe("0,123");
  });

  it("não agrupa milhares", () => {
    expect(formatDecimal(3600)).toBe("3600");
  });

  it("volta ao mesmo número quando relido", () => {
    for (const value of [-40, -35.5, 0, 0.25, 0.75, 3, 3600]) {
      expect(parseDecimal(formatDecimal(value))).toBe(value);
    }
  });
});
