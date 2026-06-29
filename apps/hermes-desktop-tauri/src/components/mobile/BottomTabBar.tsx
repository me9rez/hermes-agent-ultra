export type BottomTab = 'home' | 'voice' | 'settings'

interface BottomTabBarProps {
  active: BottomTab
  onChange: (tab: BottomTab) => void
}

const TABS: { id: BottomTab; label: string }[] = [
  { id: 'home', label: 'Home' },
  { id: 'voice', label: 'Voice' },
  { id: 'settings', label: 'Settings' },
]

export function BottomTabBar({ active, onChange }: BottomTabBarProps) {
  return (
    <nav className="terra-bottom-tabbar" aria-label="Main">
      {TABS.map((tab) => (
        <button
          key={tab.id}
          type="button"
          className={active === tab.id ? 'is-active' : undefined}
          aria-current={active === tab.id ? 'page' : undefined}
          onClick={() => onChange(tab.id)}
        >
          {tab.label}
        </button>
      ))}
    </nav>
  )
}

export default BottomTabBar
