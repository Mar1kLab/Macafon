<script lang="ts">
  // Single text field dialog (preset names, a typed folder path).
  import Dialog from './Dialog.svelte';
  import { t } from '../i18n';

  interface Props {
    open: boolean;
    title: string;
    label: string;
    initial?: string;
    confirm: string;
    maxlength?: number;
    onsubmit: (value: string) => void;
    onclose: () => void;
  }

  let { open, title, label, initial = '', confirm, maxlength = 64, onsubmit, onclose }: Props = $props();
  let text = $state('');

  $effect(() => {
    if (open) text = initial;
  });

  const valid = $derived(text.trim().length > 0 && text.trim().length <= maxlength);

  function submit(event: SubmitEvent) {
    event.preventDefault();
    if (valid) onsubmit(text.trim());
  }

  function focus(element: HTMLInputElement) {
    element.focus();
    element.select();
  }
</script>

<Dialog {open} {title} {onclose}>
  <form id="prompt-form" onsubmit={submit}>
    <label class="hud-label" for="prompt-input">{label}</label>
    <input id="prompt-input" bind:value={text} {maxlength} autocomplete="off" use:focus />
  </form>
  {#snippet actions()}
    <button class="btn" type="button" onclick={onclose}>{t('common.cancel')}</button>
    <button class="btn invert" type="submit" form="prompt-form" disabled={!valid}>{confirm}</button>
  {/snippet}
</Dialog>

<style>
  form {
    display: grid;
    gap: 8px;
  }

  input {
    height: 34px;
    padding: 0 10px;
    background: var(--bg-sunken);
    border: 1px solid var(--line);
    user-select: text;
  }

  input:focus {
    border-color: var(--accent);
    outline: none;
  }
</style>
