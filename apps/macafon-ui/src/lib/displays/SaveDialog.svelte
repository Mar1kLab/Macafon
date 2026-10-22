<script lang="ts">
  // DISP-001: save the current layout under a name and an icon.
  import Dialog from '../components/Dialog.svelte';
  import IconPicker from './IconPicker.svelte';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import Field from '../components/Field.svelte';
  import Input from '../components/Input.svelte';

  let { open, onclose }: { open: boolean; onclose: () => void } = $props();

  let name = $state('');
  let icon = $state('monitor');
  let busy = $state(false);

  $effect(() => {
    if (open) {
      name = '';
      icon = 'monitor';
    }
  });

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!name.trim() || busy) return;
    busy = true;
    const ok = await daemon.setDisplays(daemon.client.call('displays.save', { name: name.trim(), icon }));
    busy = false;
    if (ok) onclose();
  }
</script>

<Dialog {open} title={t('displays.saveTitle')} {onclose}>
  <form id="display-save" class="form" onsubmit={save}>
    <Field label={t('displays.name')}>
      <Input bind:value={name} maxlength={40} placeholder={t('displays.namePlaceholder')} autofocus />
    </Field>
    <div class="field">
      <span class="hud-label">{t('displays.icon')}</span>
      <IconPicker value={icon} onchange={(next) => (icon = next)} />
    </div>
  </form>
  {#snippet actions()}
    <button class="btn" type="button" onclick={onclose}>{t('common.cancel')}</button>
    <button class="btn invert" type="submit" form="display-save" disabled={!name.trim() || busy}>{t('displays.save')}</button>
  {/snippet}
</Dialog>

<style>
  .form { display: flex; flex-direction: column; gap: var(--space-4); }
  .field { display: flex; flex-direction: column; gap: var(--space-2); }
</style>
