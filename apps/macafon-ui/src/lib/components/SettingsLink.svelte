<script lang="ts">
  // A screen header's gear: Settings, scrolled to the section that configures this screen.
  import { Settings2 } from '@lucide/svelte';
  import { nav, type SettingsSection } from '../state/nav.svelte';
  import { t } from '../i18n';
  import { tip } from './tooltip.svelte';

  let { section }: { section: SettingsSection } = $props();
  const label = $derived(t('nav.settingsFor', { section: t(`nav.sections.${section}`) }));
</script>

<button class="btn icon ghost" aria-label={label} {@attach tip(label)} onclick={() => nav.settings(section)}>
  <Settings2 size={15} strokeWidth={1.5} />
</button>
