import { useState } from 'react'

import { BottomTabBar, type BottomTab } from '@/components/mobile/BottomTabBar'

export default function MobileApp() {
  const [tab, setTab] = useState<BottomTab>('home')

  return (
    <div className="terra-mobile-shell">
      <main className="terra-mobile-shell__content">
        {tab === 'home' && <div>Tasks</div>}
        {tab === 'voice' && <div>Voice</div>}
        {tab === 'settings' && <div>Settings</div>}
      </main>
      <BottomTabBar active={tab} onChange={setTab} />
    </div>
  )
}
