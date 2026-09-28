// Ponto de entrada do webview: monta o formulário e o painel, passa a ouvir o andamento e
// confere se o ffmpeg está instalado.

import { byId } from "./dom.ts";
import {
  checkTools,
  currentJob,
  getSettings,
  onDragHover,
  onJobEvent,
} from "./ipc.ts";
import { JobView } from "./job-view.ts";
import { describeError } from "./messages.ts";
import { SettingsForm } from "./settings-form.ts";

async function boot(): Promise<void> {
  const form = new SettingsForm(await getSettings());
  const job = new JobView((busy) => form.setDisabled(busy));
  // A escuta vem antes da pergunta pelo andamento, para nenhum evento cair no intervalo.
  await onJobEvent((event) => job.apply(event));
  await onDragHover((hovering) => job.setHovering(hovering));
  const running = await currentJob();
  if (running !== null) {
    job.resume(running);
  }
  checkTools().catch((error: unknown) => {
    byId("tools-banner-text", HTMLElement).textContent =
      describeError(error).text;
    byId("tools-banner", HTMLElement).hidden = false;
  });
}

boot().catch((error: unknown) => {
  // A montagem falhou: sem formulário nem painel, o que resta é a faixa de aviso do topo.
  const { text, detail } = describeError(error);
  const banner = document.getElementById("tools-banner");
  const bannerText = document.getElementById("tools-banner-text");
  if (banner !== null && bannerText !== null) {
    bannerText.textContent = detail === null ? text : `${text} (${detail})`;
    banner.hidden = false;
  }
});
