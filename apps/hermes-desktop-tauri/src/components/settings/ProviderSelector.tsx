import { useT } from '@/i18n/useT'

export type ProviderTier = 'smart' | 'economic' | 'local'

interface ProviderSelectorProps {
  value?: ProviderTier
  onChange?: (tier: ProviderTier) => void
  showAdvanced?: boolean
}

export function ProviderSelector({ value = 'smart', onChange, showAdvanced }: ProviderSelectorProps) {
  const t = useT('settings')
  const tiers: ProviderTier[] = ['smart', 'economic', 'local']

  return (
    <section className="terra-provider-selector">
      <h3>{t('provider.title', 'Provider tier')}</h3>
      <div className="terra-provider-selector__tiers" role="radiogroup">
        {tiers.map(tier => (
          <label key={tier}>
            <input
              type="radio"
              name="provider-tier"
              checked={value === tier}
              onChange={() => onChange?.(tier)}
            />
            {t(`provider.${tier}`, tier)}
          </label>
        ))}
      </div>
      {showAdvanced ? (
        <details className="terra-provider-selector__advanced">
          <summary>{t('provider.advanced', 'Advanced raw models')}</summary>
          <p>{t('provider.advancedHint', 'Override per-model in developer mode.')}</p>
        </details>
      ) : null}
    </section>
  )
}

export default ProviderSelector
