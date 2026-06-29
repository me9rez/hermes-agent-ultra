import { useT } from '@/i18n/useT'

export interface AccountOption {
  userId: string
  email: string
  avatarUrl?: string
}

interface AccountSwitcherProps {
  accounts?: AccountOption[]
  activeUserId?: string
  onSwitch?: (userId: string) => void
  onAdd?: () => void
  onSignOut?: () => void
}

export function AccountSwitcher({
  accounts = [],
  activeUserId,
  onSwitch,
  onAdd,
  onSignOut
}: AccountSwitcherProps) {
  const t = useT('auth')

  return (
    <div className="terra-account-switcher">
      <button type="button" className="terra-account-switcher__trigger" aria-haspopup="menu">
        {t('account.menu', 'Account')}
      </button>
      <ul className="terra-account-switcher__menu" role="menu">
        {accounts.map(account => (
          <li key={account.userId} role="none">
            <button
              type="button"
              role="menuitem"
              aria-current={account.userId === activeUserId}
              onClick={() => onSwitch?.(account.userId)}
            >
              {account.email}
            </button>
          </li>
        ))}
        <li role="none">
          <button type="button" role="menuitem" onClick={onAdd}>
            {t('account.add', 'Add account')}
          </button>
        </li>
        <li role="none">
          <button type="button" role="menuitem" onClick={onSignOut}>
            {t('account.signOut', 'Sign out')}
          </button>
        </li>
      </ul>
    </div>
  )
}

export default AccountSwitcher
