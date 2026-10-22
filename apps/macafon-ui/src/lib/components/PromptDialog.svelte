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
  let input: HTMLInputElement | undefined = $state();

  // The dialog takes focus when it opens; once it has and the value is in, select the text so
  // typing replaces it.
  $effect(() => {
    if (!open) return;
    text = initial;
    const frame = requestAnimationFrame(() => {
      input?.focus();
      input?.select();
    });
    return () => cancelAnimationFrame(frame);
  });

  const valid = $derived(text.trim().length > 0 && text.trim().length <= maxlength);

  function submit(event: SubmitEvent) {
    event.preventDefault();
    if (valid) onsubmit(text.trim());
  }
</script>

<Dialog {open} {title} {onclose}>
  <form id="prompt-form" onsubmit={submit}>
    <Field {label}>
      <Input id="prompt-input" bind:value={text} bind:element={input} {maxlength} />
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
