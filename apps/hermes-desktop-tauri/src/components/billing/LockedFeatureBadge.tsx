import { useT } from '@/i18n/useT'

export function LockedFeatureBadge() {
  const t = useT('billing')

  return (
    <span className="terra-locked-badge" title={t('locked.tooltip', 'Pro feature')}>
      {t('locked.label', 'Pro')}
    </span>
  )
}

export default LockedFeatureBadge
