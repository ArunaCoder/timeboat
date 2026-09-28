// Contrato IPC do webview: um wrapper tipado por comando do Rust, e a escuta dos eventos.
//
// Nenhum comando recebe caminho de arquivo: o soltar na janela e o seletor de arquivos são
// vistos só pelo Rust, e o que chega aqui são os eventos de andamento. A escuta do
// drag-and-drop nativo serve só ao destaque visual da área de soltar.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";

import type { JobEvent, JobSnapshot, Settings, SettingsView } from "./types.ts";

/// O nome do evento de andamento. Espelha `job::EVENT`.
const JOB_EVENT = "job";

/// As configurações correntes, os padrões e as faixas aceitas.
export function getSettings(): Promise<SettingsView> {
  return invoke<SettingsView>("get_settings");
}

/// Grava as réguas; devolve as que ficaram em vigor.
export function saveSettings(settings: Settings): Promise<Settings> {
  return invoke<Settings>("save_settings", { settings });
}

/// Volta aos padrões de fábrica e os devolve.
export function resetSettings(): Promise<Settings> {
  return invoke<Settings>("reset_settings");
}

/// Rejeita com `ffmpeg_missing` se o ffmpeg ou o ffprobe não estiverem instalados.
export function checkTools(): Promise<void> {
  return invoke<void>("check_tools");
}

/// Abre o seletor de arquivos nativo; o andamento chega pelos eventos.
export function pickMedia(): Promise<void> {
  return invoke<void>("pick_media");
}

/// Pede a interrupção do processamento em andamento.
export function cancelJob(): Promise<void> {
  return invoke<void>("cancel_job");
}

/// O processamento em andamento (arquivo e tempo decorrido), ou `null`.
export function currentJob(): Promise<JobSnapshot | null> {
  return invoke<JobSnapshot | null>("current_job");
}

/// Abre o Explorer com o último resultado selecionado.
export function revealOutput(): Promise<void> {
  return invoke<void>("reveal_output");
}

/// Passa cada evento de andamento a `handler`.
export function onJobEvent(
  handler: (event: JobEvent) => void,
): Promise<UnlistenFn> {
  return listen<JobEvent>(JOB_EVENT, (event) => handler(event.payload));
}

/// Avisa `handler` quando um arquivo arrastado entra (`true`) ou sai (`false`) da janela.
export function onDragHover(
  handler: (hovering: boolean) => void,
): Promise<UnlistenFn> {
  return getCurrentWebview().onDragDropEvent((event) => {
    const { type } = event.payload;
    handler(type === "enter" || type === "over");
  });
}
