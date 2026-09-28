import { describe, expect, it } from "vitest";

import { formatClock } from "./durations.ts";

describe("formatClock", () => {
  it("mostra minutos:segundos abaixo de uma hora", () => {
    expect(formatClock(0)).toBe("0:00");
    expect(formatClock(7)).toBe("0:07");
    expect(formatClock(125)).toBe("2:05");
    expect(formatClock(3599.4)).toBe("59:59");
  });

  it("arredonda ao segundo inteiro", () => {
    expect(formatClock(2.5)).toBe("0:03");
    expect(formatClock(59.6)).toBe("1:00");
  });

  it("passa a horas:minutos:segundos a partir de uma hora", () => {
    expect(formatClock(3600)).toBe("1:00:00");
    expect(formatClock(3725.4)).toBe("1:02:05");
  });

  it("não inventa duração para valor inválido", () => {
    expect(formatClock(Number.NaN)).toBe("—");
    expect(formatClock(-1)).toBe("—");
  });
});
