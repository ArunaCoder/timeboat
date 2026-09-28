// O formulário das quatro réguas: confere cada campo enquanto a pessoa digita e grava
// sozinho, sem botão de salvar.
//
// Um campo inválido mostra o que corrigir e não é gravado; ao sair dele, volta ao último
// valor válido. A gravação espera uma pausa curta na digitação, e sai na hora quando a
// pessoa deixa o campo — que é o que acontece ao ir buscar um arquivo no Explorer, e é
// isso que garante que o arquivo solto em seguida use o valor que está na tela.

import { byId } from "./dom.ts";
import { resetSettings, saveSettings } from "./ipc.ts";
import { describeError } from "./messages.ts";
import { formatDecimal } from "./numbers.ts";
import { checkField, FIELD_ORDER } from "./settings-fields.ts";
import type { Limits, SettingKey, Settings, SettingsView } from "./types.ts";

/// A pausa na digitação que dispara a gravação.
const SAVE_DELAY_MS = 400;

interface Field {
  readonly key: SettingKey;
  readonly input: HTMLInputElement;
  readonly error: HTMLElement;
}

function sameSettings(left: Settings, right: Settings): boolean {
  return FIELD_ORDER.every((key) => left[key] === right[key]);
}

export class SettingsForm {
  /// O que o Rust confirmou por último.
  #saved: Settings;
  /// O que a tela mostra de válido, gravado ou à espera da pausa.
  #pending: Settings;
  #timer: number | undefined;
  readonly #limits: Limits;
  readonly #fields: readonly Field[];
  readonly #fieldset = byId("settings-fieldset", HTMLFieldSetElement);
  readonly #resetButton = byId("reset-button", HTMLButtonElement);
  readonly #message = byId("settings-message", HTMLElement);

  constructor(view: SettingsView) {
    this.#saved = view.current;
    this.#pending = view.current;
    this.#limits = view.limits;
    this.#fields = FIELD_ORDER.map((key) => ({
      key,
      input: byId(key, HTMLInputElement),
      error: byId(`${key}-error`, HTMLElement),
    }));
    for (const field of this.#fields) {
      field.input.addEventListener("input", () => this.#onInput(field));
      field.input.addEventListener("blur", () => this.#onBlur(field));
    }
    byId("settings-form", HTMLFormElement).addEventListener(
      "submit",
      (event) => {
        event.preventDefault();
        this.#flush();
      },
    );
    this.#resetButton.addEventListener("click", () => this.#reset());
    this.#fill(view.current);
  }

  /// Trava o formulário enquanto um arquivo é processado: a mudança não valeria para ele.
  setDisabled(disabled: boolean): void {
    this.#fieldset.disabled = disabled;
    this.#resetButton.disabled = disabled;
  }

  #onInput(field: Field): void {
    const check = checkField(field.input.value, this.#limits[field.key]);
    if (!check.ok) {
      this.#showFieldError(field, check.reason);
      return;
    }
    this.#showFieldError(field, null);
    this.#schedule({ ...this.#pending, [field.key]: check.value });
  }

  #onBlur(field: Field): void {
    const check = checkField(field.input.value, this.#limits[field.key]);
    const value = check.ok ? check.value : this.#pending[field.key];
    field.input.value = formatDecimal(value);
    this.#showFieldError(field, null);
    this.#flush();
  }

  #schedule(next: Settings): void {
    this.#pending = next;
    window.clearTimeout(this.#timer);
    this.#timer = window.setTimeout(() => this.#flush(), SAVE_DELAY_MS);
  }

  /// Grava o que está pendente, se difere do gravado.
  #flush(): void {
    window.clearTimeout(this.#timer);
    this.#timer = undefined;
    const snapshot = this.#pending;
    if (sameSettings(snapshot, this.#saved)) {
      return;
    }
    saveSettings(snapshot).then(
      (stored) => {
        this.#saved = stored;
        // Uma edição feita durante a gravação continua pendente; só a que foi gravada é
        // trocada pela versão confirmada.
        if (this.#pending === snapshot) {
          this.#pending = stored;
        }
        this.#showMessage(null);
      },
      (error: unknown) => this.#showMessage(describeError(error).text),
    );
  }

  #reset(): void {
    window.clearTimeout(this.#timer);
    this.#timer = undefined;
    resetSettings().then(
      (stored) => {
        this.#saved = stored;
        this.#pending = stored;
        this.#fill(stored);
        this.#showMessage(null);
      },
      (error: unknown) => this.#showMessage(describeError(error).text),
    );
  }

  #fill(settings: Settings): void {
    for (const field of this.#fields) {
      field.input.value = formatDecimal(settings[field.key]);
      this.#showFieldError(field, null);
    }
  }

  #showFieldError(field: Field, reason: string | null): void {
    field.error.textContent = reason ?? "";
    field.error.hidden = reason === null;
    field.input.setAttribute("aria-invalid", String(reason !== null));
  }

  #showMessage(text: string | null): void {
    this.#message.textContent = text ?? "";
    this.#message.hidden = text === null;
  }
}
