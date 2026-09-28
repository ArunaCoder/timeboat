// A regra de cada campo do formulário, sem DOM: a ordem dos campos e a conferência do texto
// digitado contra a faixa que o Rust informa.

import { formatDecimal, parseDecimal } from "./numbers.ts";
import type { Bounds, SettingKey } from "./types.ts";

/// Os campos, na ordem em que aparecem. Cada id de `index.html` é o nome da régua; o teste
/// de `settings-fields.test.ts` confere que a lista cobre todas, uma vez cada.
export const FIELD_ORDER: readonly SettingKey[] = [
  "threshold_db",
  "min_silence_secs",
  "min_sound_secs",
  "padding_secs",
];

/// O veredito sobre o texto de um campo.
export type FieldCheck =
  | { readonly ok: true; readonly value: number }
  | { readonly ok: false; readonly reason: string };

/// Confere `text` contra `bounds`: o número, ou a frase que diz o que corrigir.
export function checkField(text: string, bounds: Bounds): FieldCheck {
  const value = parseDecimal(text);
  if (value === null) {
    return { ok: false, reason: "Digite um número, como 0,75." };
  }
  if (value < bounds.min || value > bounds.max) {
    return {
      ok: false,
      reason: `Use um valor entre ${formatDecimal(bounds.min)} e ${formatDecimal(bounds.max)}.`,
    };
  }
  return { ok: true, value };
}
