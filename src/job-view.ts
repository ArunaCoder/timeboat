// A área de soltar e o painel de andamento: o que a pessoa vê do arquivo em processamento
// e do desfecho.
//
// Durante o processamento: uma barra só, do começo ao fim (a fração vem do Rust já somando
// as duas etapas), o tempo decorrido, e a interrupção em dois passos — o primeiro clique só
// pergunta, porque interromper descarta o que já foi processado, e numa gravação longa isso
// pode ser muito tempo perdido por um clique distraído.

import { byId } from "./dom.ts";
import { formatClock } from "./durations.ts";
import { cancelJob, pickMedia, revealOutput } from "./ipc.ts";
import { describeError, describeOutcome, type Stat } from "./messages.ts";
import type { JobEvent, JobOutcome, JobSnapshot, Phase } from "./types.ts";

const PHASE_LABELS: Readonly<Record<Phase, string>> = {
  analyzing: "Procurando os silêncios…",
  rendering: "Gravando o arquivo sem os silêncios…",
};

/// Quanto tempo um aviso passageiro fica na tela.
const NOTICE_MS = 6000;

/// De quanto em quanto tempo o tempo decorrido é redesenhado.
const TICK_MS = 250;

/// Um `<div>` com um `<dt>` e um `<dd>` — o agrupamento que o `<dl>` aceita.
function statElement(stat: Stat): HTMLDivElement {
  const group = document.createElement("div");
  group.className = stat.highlight ? "stat is-highlight" : "stat";
  const label = document.createElement("dt");
  label.textContent = stat.label;
  const value = document.createElement("dd");
  value.textContent = stat.value;
  group.append(label, value);
  return group;
}

function paragraph(text: string): HTMLParagraphElement {
  const element = document.createElement("p");
  element.textContent = text;
  return element;
}

export class JobView {
  #busy = false;
  #noticeTimer: number | undefined;
  #ticker: number | undefined;
  /// O instante, no relógio de `performance.now()`, em que o processamento começou.
  #startedAt = 0;
  readonly #onBusyChange: (busy: boolean) => void;
  readonly #dropZone = byId("drop-zone", HTMLButtonElement);
  readonly #notice = byId("notice", HTMLElement);
  readonly #status = byId("status", HTMLElement);
  readonly #running = byId("status-running", HTMLElement);
  readonly #runningFile = byId("running-file", HTMLElement);
  readonly #runningPhase = byId("running-phase", HTMLElement);
  readonly #progress = byId("running-progress", HTMLProgressElement);
  readonly #percent = byId("running-percent", HTMLElement);
  readonly #elapsed = byId("running-elapsed", HTMLElement);
  readonly #stopActions = byId("stop-actions", HTMLElement);
  readonly #stopButton = byId("stop-button", HTMLButtonElement);
  readonly #stopConfirm = byId("stop-confirm", HTMLElement);
  readonly #stopConfirmButton = byId("stop-confirm-button", HTMLButtonElement);
  readonly #stopKeepButton = byId("stop-keep-button", HTMLButtonElement);
  readonly #stopping = byId("stopping", HTMLElement);
  readonly #result = byId("status-result", HTMLElement);
  readonly #resultTitle = byId("result-title", HTMLElement);
  readonly #resultLines = byId("result-lines", HTMLElement);
  readonly #resultStats = byId("result-stats", HTMLElement);
  readonly #resultDetail = byId("result-detail", HTMLDetailsElement);
  readonly #resultDetailText = byId("result-detail-text", HTMLElement);
  readonly #revealButton = byId("reveal-button", HTMLButtonElement);

  constructor(onBusyChange: (busy: boolean) => void) {
    this.#onBusyChange = onBusyChange;
    this.#dropZone.addEventListener("click", () => {
      if (!this.#busy) {
        pickMedia().catch((error: unknown) => this.#failAction(error));
      }
    });
    this.#stopButton.addEventListener("click", () => this.#askToStop(true));
    this.#stopKeepButton.addEventListener("click", () =>
      this.#askToStop(false),
    );
    this.#stopConfirmButton.addEventListener("click", () => this.#stop());
    this.#revealButton.addEventListener("click", () => {
      revealOutput().catch((error: unknown) => this.#failAction(error));
    });
  }

  /// Destaca a área de soltar enquanto um arquivo arrastado está sobre a janela.
  setHovering(hovering: boolean): void {
    this.#dropZone.classList.toggle("is-hovering", hovering && !this.#busy);
  }

  /// Retoma a exibição de um processamento já em andamento (a janela recarregou no meio),
  /// com o cronômetro no ponto em que ele está.
  resume(snapshot: JobSnapshot): void {
    this.#start(snapshot.file_name, snapshot.elapsed_secs);
  }

  apply(event: JobEvent): void {
    switch (event.kind) {
      case "started":
        this.#start(event.file_name, 0);
        return;
      case "progress":
        this.#advance(event.phase, event.fraction);
        return;
      case "rejected":
        this.#showNotice(describeError(event.error).text);
        return;
      case "finished":
      case "nothing_to_remove":
      case "all_silent":
      case "cancelled":
      case "failed":
        this.#finish(event);
        return;
    }
  }

  #setBusy(busy: boolean): void {
    this.#busy = busy;
    this.#dropZone.disabled = busy;
    this.#dropZone.classList.remove("is-hovering");
    this.#onBusyChange(busy);
  }

  #start(fileName: string, elapsedSecs: number): void {
    this.#setBusy(true);
    this.#hideNotice();
    this.#status.hidden = false;
    this.#status.setAttribute("data-tone", "progress");
    this.#result.hidden = true;
    this.#running.hidden = false;
    this.#runningFile.textContent = fileName;
    this.#runningPhase.textContent = "Lendo o arquivo…";
    // Sem `value`, a barra fica indeterminada até o primeiro progresso.
    this.#progress.removeAttribute("value");
    this.#percent.textContent = "";
    this.#askToStop(false);
    this.#stopping.hidden = true;
    this.#startClock(elapsedSecs);
  }

  #advance(phase: Phase, fraction: number): void {
    this.#runningPhase.textContent = PHASE_LABELS[phase];
    this.#progress.value = fraction;
    this.#percent.textContent = `${Math.floor(fraction * 100)}%`;
  }

  /// Mostra (ou recolhe) a pergunta de confirmação da interrupção.
  #askToStop(asking: boolean): void {
    this.#stopActions.hidden = asking;
    this.#stopConfirm.hidden = !asking;
    if (asking) {
      this.#stopKeepButton.focus();
    }
  }

  /// A interrupção confirmada: o Rust encerra o ffmpeg e apaga o parcial, e o desfecho
  /// chega como evento `cancelled`.
  #stop(): void {
    this.#stopConfirm.hidden = true;
    this.#stopActions.hidden = true;
    this.#stopping.hidden = false;
    cancelJob().catch((error: unknown) => {
      this.#stopping.hidden = true;
      this.#askToStop(false);
      this.#failAction(error);
    });
  }

  #startClock(elapsedSecs: number): void {
    this.#startedAt = performance.now() - elapsedSecs * 1000;
    this.#stopClock();
    this.#renderClock();
    this.#ticker = window.setInterval(() => this.#renderClock(), TICK_MS);
  }

  #renderClock(): void {
    const elapsed = (performance.now() - this.#startedAt) / 1000;
    this.#elapsed.textContent = formatClock(elapsed);
  }

  #stopClock(): void {
    window.clearInterval(this.#ticker);
    this.#ticker = undefined;
  }

  #finish(outcome: JobOutcome): void {
    this.#stopClock();
    this.#setBusy(false);
    const text = describeOutcome(outcome);
    this.#status.hidden = false;
    this.#status.setAttribute("data-tone", text.tone);
    this.#running.hidden = true;
    this.#result.hidden = false;
    this.#resultTitle.textContent = text.title;
    this.#resultLines.replaceChildren(...text.lines.map(paragraph));
    this.#resultStats.replaceChildren(...text.stats.map(statElement));
    this.#resultStats.hidden = text.stats.length === 0;
    this.#resultDetail.hidden = text.detail === null;
    this.#resultDetail.open = false;
    this.#resultDetailText.textContent = text.detail ?? "";
    this.#revealButton.hidden = !text.canReveal;
  }

  #failAction(error: unknown): void {
    this.#showNotice(describeError(error).text);
  }

  #showNotice(text: string): void {
    window.clearTimeout(this.#noticeTimer);
    this.#notice.textContent = text;
    this.#notice.hidden = false;
    this.#noticeTimer = window.setTimeout(() => this.#hideNotice(), NOTICE_MS);
  }

  #hideNotice(): void {
    window.clearTimeout(this.#noticeTimer);
    this.#noticeTimer = undefined;
    this.#notice.hidden = true;
  }
}
