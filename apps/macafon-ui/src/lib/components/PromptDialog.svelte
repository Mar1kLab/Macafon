<script lang="ts">
  // Single text field dialog (preset names, a typed folder path).
  import Dialog from './Dialog.svelte';
  import Field from './Field.svelte';
  import Input from './Input.svelte';
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

  function focus(element: HTMLElement) {
    if (!(element instanceof HTMLInputElement)) return;
    element.focus();
    element.select();
  }
</script>

<Dialog {open} {title} {onclose}>
  <form id="prompt-form" onsubmit={submit}>
    <Field {label}>
      <Input id="prompt-input" bind:value={text} {maxlength} {@attach focus} />
    </Field>
  </form>
  {#snippet actions()}
    <button class="btn" type="button" onclick={onclose}>{t('common.cancel')}</button>
    <button class="btn invert" type="submit" form="prompt-form" disabled={!valid}>{confirm}</button>
  {/snippet}
</Dialog>

<style>
  form {
    display: grid;
    gap: var(--space-2);
  }
</style>
